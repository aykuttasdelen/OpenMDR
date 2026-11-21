use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_MAX_VERSIONS: u32 = 50;
pub const MIN_MAX_VERSIONS: u32 = 5;
pub const MAX_MAX_VERSIONS: u32 = 200;
pub const DEFAULT_AUTOSAVE_INTERVAL_MINUTES: u32 = 3;
pub const MIN_AUTOSAVE_INTERVAL_MINUTES: u32 = 1;
pub const MAX_AUTOSAVE_INTERVAL_MINUTES: u32 = 60;
pub const MAX_SNAPSHOT_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_DIFF_LINES: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySettings {
    pub enabled: bool,
    pub max_versions_per_file: u32,
    pub autosave_interval_minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummary {
    pub id: i64,
    pub file_id: i64,
    pub created_at: i64,
    pub trigger: String,
    pub size_bytes: i64,
    pub compressed_bytes: i64,
    pub line_count: i64,
    pub word_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedFileSummary {
    pub file_id: i64,
    pub file_path: String,
    pub file_name: String,
    pub updated_at: i64,
    pub snapshot_count: i64,
    pub latest_snapshot_id: i64,
    pub latest_size_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub tag: String,
    pub old_line_no: Option<usize>,
    pub new_line_no: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffResult {
    pub lines: Vec<DiffLine>,
    pub added_count: usize,
    pub removed_count: usize,
    pub unchanged_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStats {
    pub total_files: i64,
    pub total_snapshots: i64,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackedFileRecord {
    pub id: i64,
    pub file_path: String,
    pub file_name: String,
}

/// Persistent SQLite-backed version history store and line-level diff engine.
pub struct HistoryStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl HistoryStore {
    /// Opens (or creates) the SQLite database at `db_path`, configures PRAGMAs, and applies schema v1.
    pub fn open(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }

        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;
            PRAGMA user_version = 1;

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS tracked_files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path TEXT NOT NULL UNIQUE,
                file_name TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS snapshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_id INTEGER NOT NULL REFERENCES tracked_files(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL,
                trigger TEXT NOT NULL CHECK(trigger IN ('initial', 'manual', 'autosave', 'pre_restore')),
                content_hash TEXT NOT NULL,
                content_gz BLOB NOT NULL,
                size_bytes INTEGER NOT NULL,
                compressed_bytes INTEGER NOT NULL,
                line_count INTEGER NOT NULL,
                word_count INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_snapshots_file_created
                ON snapshots(file_id, created_at DESC, id DESC);
            ",
        )
        .map_err(|e| e.to_string())?;

        conn.execute(
            "
            INSERT OR IGNORE INTO settings (key, value)
            VALUES
                ('enabled', 'true'),
                ('max_versions_per_file', ?1),
                ('autosave_interval_minutes', ?2)
            ",
            params![
                DEFAULT_MAX_VERSIONS.to_string(),
                DEFAULT_AUTOSAVE_INTERVAL_MINUTES.to_string()
            ],
        )
        .map_err(|e| e.to_string())?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path,
        })
    }

    /// Normalizes a file or directory path by trimming trailing separators except roots.
    pub fn normalize_path(path: &str) -> String {
        let trimmed = if cfg!(windows) {
            path.trim_end_matches(['/', '\\'])
        } else {
            path.trim_end_matches('/')
        };

        if trimmed.is_empty() && path.starts_with('/') {
            "/".to_string()
        } else if cfg!(windows)
            && trimmed.len() == 2
            && trimmed.as_bytes().get(1) == Some(&b':')
        {
            let sep = if path.contains('\\') { '\\' } else { '/' };
            format!("{trimmed}{sep}")
        } else if cfg!(windows) && trimmed.is_empty() && path.starts_with('\\') {
            "\\".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Reads the persisted history settings from SQLite, falling back to defaults for missing keys.
    pub fn get_settings(&self) -> Result<HistorySettings, String> {
        let conn = self.lock_conn()?;
        read_settings_from_conn(&conn)
    }

    /// Validates, saves, and applies updated settings, pruning excess snapshots across all files.
    pub fn update_settings(&self, settings: HistorySettings) -> Result<HistorySettings, String> {
        if !(MIN_MAX_VERSIONS..=MAX_MAX_VERSIONS).contains(&settings.max_versions_per_file) {
            return Err(format!(
                "max_versions_per_file must be between {MIN_MAX_VERSIONS} and {MAX_MAX_VERSIONS}"
            ));
        }
        if !(MIN_AUTOSAVE_INTERVAL_MINUTES..=MAX_AUTOSAVE_INTERVAL_MINUTES)
            .contains(&settings.autosave_interval_minutes)
        {
            return Err(format!(
                "autosave_interval_minutes must be between {MIN_AUTOSAVE_INTERVAL_MINUTES} and {MAX_AUTOSAVE_INTERVAL_MINUTES}"
            ));
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        let entries = [
            ("enabled", if settings.enabled { "true".to_string() } else { "false".to_string() }),
            ("max_versions_per_file", settings.max_versions_per_file.to_string()),
            ("autosave_interval_minutes", settings.autosave_interval_minutes.to_string()),
        ];

        for (key, value) in entries {
            tx.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| e.to_string())?;
        }

        let mut stmt = tx
            .prepare("SELECT id FROM tracked_files")
            .map_err(|e| e.to_string())?;
        let file_ids: Vec<i64> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        for file_id in file_ids {
            prune_file_snapshots(&tx, file_id, settings.max_versions_per_file)?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(settings)
    }

    /// Returns true if at least one snapshot exists for the given file path.
    pub fn has_snapshots(&self, file_path: &str) -> Result<bool, String> {
        let norm_path = Self::normalize_path(file_path);
        let conn = self.lock_conn()?;
        let exists: i64 = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM snapshots s
                    INNER JOIN tracked_files f ON f.id = s.file_id
                    WHERE f.file_path = ?1
                )",
                params![norm_path],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok(exists != 0)
    }

    /// Records a snapshot using the current wall-clock time in Unix milliseconds.
    pub fn record_snapshot(
        &self,
        file_path: &str,
        content: &str,
        trigger: &str,
    ) -> Result<Option<i64>, String> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis() as i64;
        self.record_snapshot_at(file_path, content, trigger, now_ms)
    }

    /// Records a snapshot at an explicit timestamp, enforcing size limits, dedup, throttling, and pruning.
    pub fn record_snapshot_at(
        &self,
        file_path: &str,
        content: &str,
        trigger: &str,
        now_ms: i64,
    ) -> Result<Option<i64>, String> {
        if !matches!(trigger, "initial" | "manual" | "autosave" | "pre_restore") {
            return Err(format!("Invalid snapshot trigger: {trigger}"));
        }

        let size_bytes = content.len();
        if size_bytes > MAX_SNAPSHOT_BYTES {
            eprintln!(
                "[history_store] skipping snapshot for {}: {} bytes exceeds {} byte limit",
                file_path, size_bytes, MAX_SNAPSHOT_BYTES
            );
            return Ok(None);
        }

        let norm_path = Self::normalize_path(file_path);
        if norm_path.is_empty() {
            return Err("File path cannot be empty".to_string());
        }

        let content_hash = compute_sha256_hex(content);
        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        let settings = read_settings_from_conn(&tx)?;
        if !settings.enabled {
            return Ok(None);
        }

        let existing_file_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM tracked_files WHERE file_path = ?1",
                params![&norm_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        if let Some(file_id) = existing_file_id {
            let latest: Option<(String, i64)> = tx
                .query_row(
                    "SELECT content_hash, created_at
                     FROM snapshots
                     WHERE file_id = ?1
                     ORDER BY created_at DESC, id DESC
                     LIMIT 1",
                    params![file_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(|e| e.to_string())?;

            if let Some((latest_hash, latest_created_at)) = latest {
                if latest_hash == content_hash {
                    return Ok(None);
                }
                if trigger == "autosave" {
                    let interval_ms = (settings.autosave_interval_minutes as i64) * 60_000;
                    if now_ms.saturating_sub(latest_created_at) < interval_ms {
                        return Ok(None);
                    }
                }
            }
        }

        let compressed = compress_content(content)?;
        let compressed_bytes = compressed.len() as i64;
        let line_count = content.lines().count() as i64;
        let word_count = content.split_whitespace().count() as i64;
        let file_name = extract_file_name(&norm_path);

        let file_id = match existing_file_id {
            Some(id) => {
                tx.execute(
                    "UPDATE tracked_files SET file_name = ?1, updated_at = ?2 WHERE id = ?3",
                    params![file_name, now_ms, id],
                )
                .map_err(|e| e.to_string())?;
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO tracked_files (file_path, file_name, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?3)",
                    params![norm_path, file_name, now_ms],
                )
                .map_err(|e| e.to_string())?;
                tx.last_insert_rowid()
            }
        };

        tx.execute(
            "INSERT INTO snapshots (
                file_id, created_at, trigger, content_hash, content_gz,
                size_bytes, compressed_bytes, line_count, word_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                file_id,
                now_ms,
                trigger,
                content_hash,
                compressed,
                size_bytes as i64,
                compressed_bytes,
                line_count,
                word_count
            ],
        )
        .map_err(|e| e.to_string())?;
        let snapshot_id = tx.last_insert_rowid();

        prune_file_snapshots(&tx, file_id, settings.max_versions_per_file)?;
        tx.commit().map_err(|e| e.to_string())?;

        Ok(Some(snapshot_id))
    }

    /// Lists snapshot summaries for a file path, ordered newest first.
    pub fn list_snapshots(&self, file_path: &str) -> Result<Vec<SnapshotSummary>, String> {
        let norm_path = Self::normalize_path(file_path);
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.created_at, s.trigger, s.size_bytes,
                        s.compressed_bytes, s.line_count, s.word_count
                 FROM snapshots s
                 INNER JOIN tracked_files f ON f.id = s.file_id
                 WHERE f.file_path = ?1
                 ORDER BY s.created_at DESC, s.id DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![norm_path], map_snapshot_summary)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// Lists snapshot summaries for a tracked file ID, ordered newest first.
    pub fn list_snapshots_by_file_id(&self, file_id: i64) -> Result<Vec<SnapshotSummary>, String> {
        let conn = self.lock_conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, file_id, created_at, trigger, size_bytes,
                        compressed_bytes, line_count, word_count
                 FROM snapshots
                 WHERE file_id = ?1
                 ORDER BY created_at DESC, id DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![file_id], map_snapshot_summary)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// Reads and decompresses the UTF-8 content for a snapshot by ID.
    pub fn get_snapshot_content(&self, snapshot_id: i64) -> Result<String, String> {
        let compressed: Vec<u8> = {
            let conn = self.lock_conn()?;
            conn.query_row(
                "SELECT content_gz FROM snapshots WHERE id = ?1",
                params![snapshot_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Snapshot {snapshot_id} not found"))?
        };
        decompress_content(&compressed)
    }

    /// Verifies that `snapshot_id` belongs to `file_path` before decompressing and returning content.
    pub fn get_snapshot_content_for_file(
        &self,
        file_path: &str,
        snapshot_id: i64,
    ) -> Result<String, String> {
        let norm_path = Self::normalize_path(file_path);
        let compressed: Vec<u8> = {
            let conn = self.lock_conn()?;
            conn.query_row(
                "SELECT s.content_gz
                 FROM snapshots s
                 INNER JOIN tracked_files f ON f.id = s.file_id
                 WHERE s.id = ?1 AND f.file_path = ?2",
                params![snapshot_id, norm_path],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!("Snapshot {snapshot_id} not found for file '{norm_path}'")
            })?
        };
        decompress_content(&compressed)
    }

    /// Fetches the tracked file record for `file_id`.
    pub fn get_tracked_file(&self, file_id: i64) -> Result<TrackedFileRecord, String> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, file_path, file_name FROM tracked_files WHERE id = ?1",
            params![file_id],
            |row| {
                Ok(TrackedFileRecord {
                    id: row.get(0)?,
                    file_path: row.get(1)?,
                    file_name: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Tracked file {file_id} not found"))
    }

    /// Computes a line-level diff from the stored snapshot (`old`) to `current_content` (`new`).
    pub fn diff_snapshot(
        &self,
        snapshot_id: i64,
        current_content: &str,
    ) -> Result<DiffResult, String> {
        let old_content = self.get_snapshot_content(snapshot_id)?;
        let diff = TextDiff::from_lines(old_content.as_str(), current_content);

        let mut lines = Vec::new();
        let mut added_count = 0usize;
        let mut removed_count = 0usize;
        let mut unchanged_count = 0usize;
        let mut truncated = false;

        for change in diff.iter_all_changes() {
            let tag = match change.tag() {
                ChangeTag::Equal => {
                    unchanged_count += 1;
                    "equal"
                }
                ChangeTag::Insert => {
                    added_count += 1;
                    "insert"
                }
                ChangeTag::Delete => {
                    removed_count += 1;
                    "delete"
                }
            };

            if lines.len() < MAX_DIFF_LINES {
                lines.push(DiffLine {
                    tag: tag.to_string(),
                    old_line_no: change.old_index().map(|idx| idx + 1),
                    new_line_no: change.new_index().map(|idx| idx + 1),
                    content: change.value().trim_end_matches(['\r', '\n']).to_string(),
                });
            } else {
                truncated = true;
            }
        }

        Ok(DiffResult {
            lines,
            added_count,
            removed_count,
            unchanged_count,
            truncated,
        })
    }

    /// Updates tracked file paths after a file or directory rename, merging collisions cleanly.
    pub fn handle_rename(&self, old_path: &str, new_path: &str) -> Result<(), String> {
        let old_norm = Self::normalize_path(old_path);
        let new_norm = Self::normalize_path(new_path);
        if old_norm.is_empty() || new_norm.is_empty() || old_norm == new_norm {
            return Ok(());
        }

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let settings = read_settings_from_conn(&tx)?;

        if let Some((old_id, old_updated_at)) = tx
            .query_row(
                "SELECT id, updated_at FROM tracked_files WHERE file_path = ?1",
                params![&old_norm],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?
        {
            move_or_merge_file(
                &tx,
                old_id,
                old_updated_at,
                &new_norm,
                settings.max_versions_per_file,
            )?;
        }

        if !old_norm.ends_with('/') && !old_norm.ends_with('\\') {
            let prefix_slash = format!("{old_norm}/");
            let prefix_backslash = format!("{old_norm}\\");

            let mut stmt = tx
                .prepare("SELECT id, file_path, updated_at FROM tracked_files")
                .map_err(|e| e.to_string())?;
            let rows: Vec<(i64, String, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            drop(stmt);

            for (child_id, child_path, child_updated_at) in rows {
                let target_raw = if let Some(rest) = child_path.strip_prefix(&prefix_slash) {
                    Some(join_renamed_child(&new_norm, '/', rest))
                } else {
                    child_path
                        .strip_prefix(&prefix_backslash)
                        .map(|rest| join_renamed_child(&new_norm, '\\', rest))
                };

                if let Some(raw) = target_raw {
                    let target_norm = Self::normalize_path(&raw);
                    move_or_merge_file(
                        &tx,
                        child_id,
                        child_updated_at,
                        &target_norm,
                        settings.max_versions_per_file,
                    )?;
                }
            }
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Updates a tracked file's path and filename (e.g., after recovering a deleted file to a new path).
    pub fn update_tracked_file_path(&self, file_id: i64, new_path: &str) -> Result<(), String> {
        let new_norm = Self::normalize_path(new_path);
        if new_norm.is_empty() {
            return Err("New path cannot be empty".to_string());
        }
        let new_name = extract_file_name(&new_norm);

        let mut conn = self.lock_conn()?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let settings = read_settings_from_conn(&tx)?;

        let current_updated_at: i64 = tx
            .query_row(
                "SELECT updated_at FROM tracked_files WHERE id = ?1",
                params![file_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Tracked file {file_id} not found"))?;

        let existing_target: Option<(i64, i64)> = tx
            .query_row(
                "SELECT id, updated_at FROM tracked_files WHERE file_path = ?1",
                params![&new_norm],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        if let Some((other_id, other_updated_at)) = existing_target
            && other_id != file_id
        {
            merge_file_into(
                &tx,
                file_id,
                other_id,
                &new_norm,
                current_updated_at.max(other_updated_at),
                settings.max_versions_per_file,
            )?;
        } else {
            tx.execute(
                "UPDATE tracked_files SET file_path = ?1, file_name = ?2 WHERE id = ?3",
                params![new_norm, new_name, file_id],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Lists tracked files that have at least one snapshot and no longer exist on disk.
    pub fn list_deleted_files(&self) -> Result<Vec<DeletedFileSummary>, String> {
        let mut candidates = {
            let conn = self.lock_conn()?;
            let mut stmt = conn
                .prepare(
                    "SELECT
                        f.id,
                        f.file_path,
                        f.file_name,
                        ls.created_at AS updated_at,
                        counts.snapshot_count,
                        ls.id AS latest_snapshot_id,
                        ls.size_bytes AS latest_size_bytes
                     FROM tracked_files f
                     INNER JOIN (
                         SELECT file_id, COUNT(*) AS snapshot_count
                         FROM snapshots
                         GROUP BY file_id
                     ) counts ON counts.file_id = f.id
                     INNER JOIN snapshots ls ON ls.id = (
                         SELECT s2.id
                         FROM snapshots s2
                         WHERE s2.file_id = f.id
                         ORDER BY s2.created_at DESC, s2.id DESC
                         LIMIT 1
                     )
                     ORDER BY ls.created_at DESC, f.id DESC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(DeletedFileSummary {
                    file_id: row.get(0)?,
                    file_path: row.get(1)?,
                    file_name: row.get(2)?,
                    updated_at: row.get(3)?,
                    snapshot_count: row.get(4)?,
                    latest_snapshot_id: row.get(5)?,
                    latest_size_bytes: row.get(6)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
        };

        candidates.retain(|item| !Path::new(&item.file_path).exists());
        Ok(candidates)
    }

    /// Permanently deletes a tracked file and all its snapshots via cascade.
    pub fn delete_file_history(&self, file_id: i64) -> Result<(), String> {
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM tracked_files WHERE id = ?1", params![file_id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Clears all tracked files and snapshots while preserving settings, then reclaims disk space.
    pub fn clear_all(&self) -> Result<(), String> {
        let conn = self.lock_conn()?;
        conn.execute("DELETE FROM tracked_files", [])
            .map_err(|e| e.to_string())?;
        conn.execute_batch("VACUUM;").map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Returns aggregate counts of tracked files, snapshots, and the database file size in bytes.
    pub fn get_stats(&self) -> Result<HistoryStats, String> {
        let (total_files, total_snapshots) = {
            let conn = self.lock_conn()?;
            let total_files: i64 = conn
                .query_row("SELECT COUNT(*) FROM tracked_files", [], |row| row.get(0))
                .map_err(|e| e.to_string())?;
            let total_snapshots: i64 = conn
                .query_row("SELECT COUNT(*) FROM snapshots", [], |row| row.get(0))
                .map_err(|e| e.to_string())?;
            (total_files, total_snapshots)
        };
        let db_size_bytes = fs::metadata(&self.db_path).map(|m| m.len()).unwrap_or(0);

        Ok(HistoryStats {
            total_files,
            total_snapshots,
            db_size_bytes,
        })
    }

    fn lock_conn(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.conn
            .lock()
            .map_err(|e| format!("History DB mutex poisoned: {e}"))
    }
}

fn read_settings_from_conn(conn: &Connection) -> Result<HistorySettings, String> {
    let mut settings = HistorySettings {
        enabled: true,
        max_versions_per_file: DEFAULT_MAX_VERSIONS,
        autosave_interval_minutes: DEFAULT_AUTOSAVE_INTERVAL_MINUTES,
    };

    let mut stmt = conn
        .prepare("SELECT key, value FROM settings")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;

    for row in rows {
        let (key, value) = row.map_err(|e| e.to_string())?;
        match key.as_str() {
            "enabled" => {
                if let Ok(parsed) = value.parse::<bool>() {
                    settings.enabled = parsed;
                }
            }
            "max_versions_per_file" => {
                if let Ok(parsed) = value.parse::<u32>() {
                    settings.max_versions_per_file = parsed;
                }
            }
            "autosave_interval_minutes" => {
                if let Ok(parsed) = value.parse::<u32>() {
                    settings.autosave_interval_minutes = parsed;
                }
            }
            _ => {}
        }
    }

    Ok(settings)
}

fn prune_file_snapshots(
    conn: &Connection,
    file_id: i64,
    max_versions: u32,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM snapshots
         WHERE file_id = ?1
           AND id NOT IN (
               SELECT id
               FROM snapshots
               WHERE file_id = ?1
               ORDER BY created_at DESC, id DESC
               LIMIT ?2
           )",
        params![file_id, max_versions as i64],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn merge_file_into(
    conn: &Connection,
    keep_id: i64,
    drop_id: i64,
    new_norm_path: &str,
    merged_updated_at: i64,
    max_versions: u32,
) -> Result<(), String> {
    let new_file_name = extract_file_name(new_norm_path);
    conn.execute(
        "UPDATE snapshots SET file_id = ?1 WHERE file_id = ?2",
        params![keep_id, drop_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM tracked_files WHERE id = ?1", params![drop_id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE tracked_files SET file_path = ?1, file_name = ?2, updated_at = ?3 WHERE id = ?4",
        params![new_norm_path, new_file_name, merged_updated_at, keep_id],
    )
    .map_err(|e| e.to_string())?;
    prune_file_snapshots(conn, keep_id, max_versions)
}

fn move_or_merge_file(
    conn: &Connection,
    old_id: i64,
    old_updated_at: i64,
    new_norm_path: &str,
    max_versions: u32,
) -> Result<(), String> {
    let new_file_name = extract_file_name(new_norm_path);
    let target: Option<(i64, i64)> = conn
        .query_row(
            "SELECT id, updated_at FROM tracked_files WHERE file_path = ?1",
            params![new_norm_path],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;

    match target {
        Some((target_id, target_updated_at)) if target_id != old_id => {
            merge_file_into(
                conn,
                target_id,
                old_id,
                new_norm_path,
                old_updated_at.max(target_updated_at),
                max_versions,
            )?;
        }
        _ => {
            conn.execute(
                "UPDATE tracked_files SET file_path = ?1, file_name = ?2 WHERE id = ?3",
                params![new_norm_path, new_file_name, old_id],
            )
            .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}

fn join_renamed_child(new_root: &str, sep: char, rest: &str) -> String {
    if new_root == "/" || new_root.ends_with('/') || new_root.ends_with('\\') {
        format!("{new_root}{rest}")
    } else {
        format!("{new_root}{sep}{rest}")
    }
}

fn extract_file_name(norm_path: &str) -> String {
    Path::new(norm_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(norm_path)
        .to_string()
}

fn compute_sha256_hex(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn compress_content(content: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(content.as_bytes())
        .map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

fn decompress_content(compressed: &[u8]) -> Result<String, String> {
    let mut decoder = GzDecoder::new(compressed);
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).map_err(|e| e.to_string())?;
    String::from_utf8(out).map_err(|e| e.to_string())
}

fn map_snapshot_summary(row: &Row<'_>) -> rusqlite::Result<SnapshotSummary> {
    Ok(SnapshotSummary {
        id: row.get(0)?,
        file_id: row.get(1)?,
        created_at: row.get(2)?,
        trigger: row.get(3)?,
        size_bytes: row.get(4)?,
        compressed_bytes: row.get(5)?,
        line_count: row.get(6)?,
        word_count: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_schema_and_default_settings() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("nested").join("history.db");
        let store = HistoryStore::open(db_path.clone()).unwrap();

        let settings = store.get_settings().unwrap();
        assert_eq!(
            settings,
            HistorySettings {
                enabled: true,
                max_versions_per_file: DEFAULT_MAX_VERSIONS,
                autosave_interval_minutes: DEFAULT_AUTOSAVE_INTERVAL_MINUTES,
            }
        );

        let conn = store.conn.lock().unwrap();
        let user_version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        let journal_mode: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();

        assert_eq!(user_version, 1);
        assert_eq!(journal_mode.to_lowercase(), "wal");
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn test_settings_validation_and_immediate_prune() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        for i in 1..=12 {
            let snap_id = store
                .record_snapshot_at("/tmp/a.md", &format!("content v{i}"), "manual", i * 1_000)
                .unwrap();
            assert!(snap_id.is_some());
        }
        assert_eq!(store.list_snapshots("/tmp/a.md").unwrap().len(), 12);

        let updated = store
            .update_settings(HistorySettings {
                enabled: true,
                max_versions_per_file: 5,
                autosave_interval_minutes: 3,
            })
            .unwrap();
        assert_eq!(updated.max_versions_per_file, 5);

        let remaining = store.list_snapshots("/tmp/a.md").unwrap();
        assert_eq!(remaining.len(), 5);
        assert_eq!(remaining[0].created_at, 12_000);
        assert_eq!(remaining[4].created_at, 8_000);
        assert_eq!(
            store.get_snapshot_content(remaining[0].id).unwrap(),
            "content v12"
        );
        assert_eq!(
            store.get_snapshot_content(remaining[4].id).unwrap(),
            "content v8"
        );

        assert!(store
            .update_settings(HistorySettings {
                enabled: true,
                max_versions_per_file: 4,
                autosave_interval_minutes: 3,
            })
            .is_err());
        assert!(store
            .update_settings(HistorySettings {
                enabled: true,
                max_versions_per_file: 201,
                autosave_interval_minutes: 3,
            })
            .is_err());
        assert!(store
            .update_settings(HistorySettings {
                enabled: true,
                max_versions_per_file: 5,
                autosave_interval_minutes: 0,
            })
            .is_err());
        assert!(store
            .update_settings(HistorySettings {
                enabled: true,
                max_versions_per_file: 5,
                autosave_interval_minutes: 61,
            })
            .is_err());

        // Invalid updates must not overwrite existing settings
        assert_eq!(store.get_settings().unwrap(), updated);
    }

    #[test]
    fn test_hybrid_throttling_dedup_and_pre_restore() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        let id1 = store
            .record_snapshot_at("/tmp/doc.md", "v1", "initial", 1_000)
            .unwrap();
        assert!(id1.is_some());

        // SHA-256 dedup
        let id_dup = store
            .record_snapshot_at("/tmp/doc.md", "v1", "manual", 2_000)
            .unwrap();
        assert!(id_dup.is_none());

        // Within 3 min (180_000 ms) window from 1_000
        let id_throttled = store
            .record_snapshot_at("/tmp/doc.md", "v2", "autosave", 60_000)
            .unwrap();
        assert!(id_throttled.is_none());

        // Past 3 min window (181_000 - 1_000 == 180_000)
        let id2 = store
            .record_snapshot_at("/tmp/doc.md", "v2", "autosave", 181_000)
            .unwrap();
        assert!(id2.is_some());

        // Manual ignores throttle
        let id3 = store
            .record_snapshot_at("/tmp/doc.md", "v3", "manual", 182_000)
            .unwrap();
        assert!(id3.is_some());

        // pre_restore ignores throttle
        let id4 = store
            .record_snapshot_at("/tmp/doc.md", "v4 unsaved", "pre_restore", 183_000)
            .unwrap();
        assert!(id4.is_some());

        // pre_restore still deduplicates
        let id4_dup = store
            .record_snapshot_at("/tmp/doc.md", "v4 unsaved", "pre_restore", 184_000)
            .unwrap();
        assert!(id4_dup.is_none());

        let snaps = store.list_snapshots("/tmp/doc.md").unwrap();
        assert_eq!(snaps.len(), 4);
        assert_eq!(snaps[0].trigger, "pre_restore");
        assert_eq!(
            store.get_snapshot_content(snaps[0].id).unwrap(),
            "v4 unsaved"
        );
    }

    #[test]
    fn test_oversized_snapshot_skipped_and_disabled_setting() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        let oversized = "x".repeat(MAX_SNAPSHOT_BYTES + 1);
        let res = store
            .record_snapshot_at("/tmp/big.md", &oversized, "manual", 1_000)
            .unwrap();
        assert!(res.is_none());
        assert!(!store.has_snapshots("/tmp/big.md").unwrap());
        assert!(store.list_snapshots("/tmp/big.md").unwrap().is_empty());

        store
            .update_settings(HistorySettings {
                enabled: false,
                max_versions_per_file: DEFAULT_MAX_VERSIONS,
                autosave_interval_minutes: DEFAULT_AUTOSAVE_INTERVAL_MINUTES,
            })
            .unwrap();

        assert!(store
            .record_snapshot_at("/tmp/disabled.md", "hello", "manual", 2_000)
            .unwrap()
            .is_none());
        assert!(store
            .record_snapshot_at("/tmp/disabled.md", "hello 2", "autosave", 300_000)
            .unwrap()
            .is_none());
        assert!(!store.has_snapshots("/tmp/disabled.md").unwrap());
    }

    #[test]
    fn test_rename_file_collision_merge_and_directory_prefix_boundary() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        for (i, ts) in [1_000_i64, 3_000, 5_000].iter().enumerate() {
            store
                .record_snapshot_at("/w/a.md", &format!("a-{i}"), "manual", *ts)
                .unwrap();
        }
        for (i, ts) in [2_000_i64, 4_000, 6_000, 7_000].iter().enumerate() {
            store
                .record_snapshot_at("/w/b.md", &format!("b-{i}"), "manual", *ts)
                .unwrap();
        }

        store
            .update_settings(HistorySettings {
                enabled: true,
                max_versions_per_file: 5,
                autosave_interval_minutes: 3,
            })
            .unwrap();

        store.handle_rename("/w/a.md", "/w/b.md").unwrap();

        assert!(store.list_snapshots("/w/a.md").unwrap().is_empty());
        let merged = store.list_snapshots("/w/b.md").unwrap();
        assert_eq!(merged.len(), 5);
        let merged_timestamps: Vec<i64> = merged.iter().map(|s| s.created_at).collect();
        assert_eq!(merged_timestamps, vec![7_000, 6_000, 5_000, 4_000, 3_000]);

        store
            .record_snapshot_at("/w/docs/n1.md", "note 1", "manual", 10_000)
            .unwrap();
        store
            .record_snapshot_at("/w/docs2/n2.md", "note 2", "manual", 11_000)
            .unwrap();

        store.handle_rename("/w/docs", "/w/notes").unwrap();

        assert_eq!(store.list_snapshots("/w/notes/n1.md").unwrap().len(), 1);
        assert!(store.list_snapshots("/w/docs/n1.md").unwrap().is_empty());
        assert_eq!(store.list_snapshots("/w/docs2/n2.md").unwrap().len(), 1);
    }

    #[test]
    fn test_deleted_files_listing_and_clear_all() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        let keep_path = dir.path().join("keep.md");
        let gone_path = dir.path().join("gone.md");
        fs::write(&keep_path, "keep me").unwrap();
        fs::write(&gone_path, "delete me").unwrap();

        store
            .record_snapshot_at(keep_path.to_str().unwrap(), "keep me", "manual", 1_000)
            .unwrap();
        store
            .record_snapshot_at(gone_path.to_str().unwrap(), "delete me", "manual", 2_000)
            .unwrap();

        fs::remove_file(&gone_path).unwrap();

        let deleted = store.list_deleted_files().unwrap();
        assert_eq!(deleted.len(), 1);
        assert_eq!(
            deleted[0].file_path,
            HistoryStore::normalize_path(gone_path.to_str().unwrap())
        );
        assert_eq!(deleted[0].file_name, "gone.md");
        assert_eq!(deleted[0].snapshot_count, 1);

        let stats = store.get_stats().unwrap();
        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.total_snapshots, 2);
        assert!(stats.db_size_bytes > 0);

        store.clear_all().unwrap();
        let cleared_stats = store.get_stats().unwrap();
        assert_eq!(cleared_stats.total_files, 0);
        assert_eq!(cleared_stats.total_snapshots, 0);
    }

    #[test]
    fn test_diff_snapshot_and_truncation() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        let snap_id = store
            .record_snapshot_at("/tmp/diff.md", "line1\nline2\nline3\n", "manual", 1_000)
            .unwrap()
            .unwrap();

        let diff = store
            .diff_snapshot(snap_id, "line1\nline2-mod\nline3\nline4\n")
            .unwrap();
        assert_eq!(diff.added_count, 2);
        assert_eq!(diff.removed_count, 1);
        assert_eq!(diff.unchanged_count, 2);
        assert!(!diff.truncated);
        assert_eq!(
            diff.lines,
            vec![
                DiffLine {
                    tag: "equal".into(),
                    old_line_no: Some(1),
                    new_line_no: Some(1),
                    content: "line1".into(),
                },
                DiffLine {
                    tag: "delete".into(),
                    old_line_no: Some(2),
                    new_line_no: None,
                    content: "line2".into(),
                },
                DiffLine {
                    tag: "insert".into(),
                    old_line_no: None,
                    new_line_no: Some(2),
                    content: "line2-mod".into(),
                },
                DiffLine {
                    tag: "equal".into(),
                    old_line_no: Some(3),
                    new_line_no: Some(3),
                    content: "line3".into(),
                },
                DiffLine {
                    tag: "insert".into(),
                    old_line_no: None,
                    new_line_no: Some(4),
                    content: "line4".into(),
                },
            ]
        );

        let mut large_old = String::new();
        for i in 1..=50_050 {
            large_old.push_str(&format!("line {i}\n"));
        }
        let large_new = large_old.replacen("line 1\n", "line 1 modified\n", 1);

        let large_id = store
            .record_snapshot_at("/tmp/large.md", &large_old, "manual", 2_000)
            .unwrap()
            .unwrap();
        let large_diff = store.diff_snapshot(large_id, &large_new).unwrap();
        assert!(large_diff.truncated);
        assert!(large_diff.lines.len() <= MAX_DIFF_LINES);
    }

    #[test]
    fn test_get_snapshot_content_for_file_rejects_cross_file_mismatch() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        let snap_a = store
            .record_snapshot_at("/tmp/file_a.md", "content a", "manual", 1_000)
            .unwrap()
            .unwrap();
        let snap_b = store
            .record_snapshot_at("/tmp/file_b.md", "content b", "manual", 2_000)
            .unwrap()
            .unwrap();

        assert_eq!(
            store
                .get_snapshot_content_for_file("/tmp/file_a.md/", snap_a)
                .unwrap(),
            "content a"
        );
        assert!(store
            .get_snapshot_content_for_file("/tmp/file_a.md", snap_b)
            .is_err());
        assert!(store
            .get_snapshot_content_for_file("/tmp/file_a.md", 999_999)
            .is_err());
    }

    #[test]
    fn test_tracked_file_helpers_and_path_normalization() {
        assert_eq!(HistoryStore::normalize_path("/"), "/");
        assert_eq!(HistoryStore::normalize_path("///"), "/");
        assert_eq!(HistoryStore::normalize_path("/w/docs/"), "/w/docs");
        assert_eq!(HistoryStore::normalize_path("/w/docs///"), "/w/docs");

        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();

        let snap_id = store
            .record_snapshot("/tmp/helper.md", "hello world\nline 2", "manual")
            .unwrap()
            .unwrap();

        let snaps = store.list_snapshots("/tmp/helper.md").unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].id, snap_id);
        assert_eq!(snaps[0].line_count, 2);
        assert_eq!(snaps[0].word_count, 4);

        let file_id = snaps[0].file_id;
        assert_eq!(store.list_snapshots_by_file_id(file_id).unwrap(), snaps);

        let tracked = store.get_tracked_file(file_id).unwrap();
        assert_eq!(tracked.file_path, "/tmp/helper.md");
        assert_eq!(tracked.file_name, "helper.md");

        store
            .update_tracked_file_path(file_id, "/tmp/recovered.md/")
            .unwrap();
        let updated = store.get_tracked_file(file_id).unwrap();
        assert_eq!(updated.file_path, "/tmp/recovered.md");
        assert_eq!(updated.file_name, "recovered.md");

        // Collision merge keeps `file_id` and preserves all snapshots
        store
            .record_snapshot_at("/tmp/collision.md", "existing target", "manual", 100)
            .unwrap();
        store
            .update_tracked_file_path(file_id, "/tmp/collision.md")
            .unwrap();
        assert_eq!(
            store.get_tracked_file(file_id).unwrap().file_path,
            "/tmp/collision.md"
        );
        assert_eq!(store.list_snapshots_by_file_id(file_id).unwrap().len(), 2);

        store.delete_file_history(file_id).unwrap();
        assert!(store.get_tracked_file(file_id).is_err());
        assert!(store.list_snapshots_by_file_id(file_id).unwrap().is_empty());
    }
}
