use crate::commands::fs_ops::{resolve_available_path, validate_single_component};
use crate::commands::image_ops::{allow_asset_dir, register_asset_search_dir};
use crate::history_store::{
    DeletedFileSummary, DiffResult, HistorySettings, HistoryStats, HistoryStore, SnapshotSummary,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, State};
use tokio::io::AsyncWriteExt;

/// Managed Tauri state wrapping the optional version history SQLite store.
pub struct HistoryState(pub Option<Arc<HistoryStore>>);

impl HistoryState {
    /// Extracts the initialized `HistoryStore` handle or returns a clean error if unavailable.
    pub fn store(&self) -> Result<Arc<HistoryStore>, String> {
        self.0
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| "History store is unavailable".to_string())
    }
}

/// Result returned after recovering a deleted file to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverFileResult {
    pub file_path: String,
    pub file_name: String,
    pub content: String,
}

async fn run_blocking_store<T, F>(store: Arc<HistoryStore>, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&HistoryStore) -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || task(&store))
        .await
        .map_err(|e| format!("History background task failed: {e}"))?
}

/// Restores a snapshot to disk after saving current unsaved edits as a `pre_restore` snapshot.
pub async fn restore_snapshot_inner(
    store: Arc<HistoryStore>,
    file_path: String,
    snapshot_id: i64,
    current_content: String,
) -> Result<String, String> {
    let db_file_path = file_path.clone();
    let target_content = run_blocking_store(store, move |s| {
        // Verify snapshot ownership for this file before recording pre_restore.
        let target = s.get_snapshot_content_for_file(&db_file_path, snapshot_id)?;
        s.record_snapshot(&db_file_path, &current_content, "pre_restore")?;
        Ok(target)
    })
    .await?;

    let target_path = Path::new(&file_path);
    if let Some(parent) = target_path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create parent directory: {e}"))?;
    }

    tokio::fs::write(target_path, &target_content)
        .await
        .map_err(|e| format!("Failed to write restored file: {e}"))?;

    Ok(target_content)
}

/// Recovers a deleted file from a snapshot to its original directory or a chosen target directory.
pub async fn recover_file_inner(
    store: Arc<HistoryStore>,
    file_id: i64,
    snapshot_id: i64,
    target_dir: Option<String>,
) -> Result<RecoverFileResult, String> {
    let (tracked, content) = run_blocking_store(Arc::clone(&store), move |s| {
        let tracked = s.get_tracked_file(file_id)?;
        validate_single_component(&tracked.file_name)?;
        let content = s.get_snapshot_content_for_file(&tracked.file_path, snapshot_id)?;
        Ok((tracked, content))
    })
    .await?;

    let base_dir = match target_dir {
        Some(dir) => {
            if dir.trim().is_empty() {
                return Err("Target directory cannot be empty".to_string());
            }
            PathBuf::from(dir)
        }
        None => {
            let original = Path::new(&tracked.file_path);
            match original.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                _ => return Err("Tracked file has no valid parent directory".to_string()),
            }
        }
    };

    tokio::fs::create_dir_all(&base_dir)
        .await
        .map_err(|e| format!("Failed to create target directory: {e}"))?;

    let base_dir_for_resolve = base_dir.clone();
    let file_name_for_resolve = tracked.file_name.clone();
    let dest_path = tokio::task::spawn_blocking(move || {
        resolve_available_path(&base_dir_for_resolve, &file_name_for_resolve)
    })
    .await
    .map_err(|e| format!("Failed to resolve destination path: {e}"))??;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&dest_path)
        .await
        .map_err(|e| format!("Failed to create recovered file: {e}"))?;
    file.write_all(content.as_bytes())
        .await
        .map_err(|e| format!("Failed to write recovered file: {e}"))?;
    file.flush()
        .await
        .map_err(|e| format!("Failed to flush recovered file: {e}"))?;

    let dest_str = dest_path.to_string_lossy().to_string();
    let final_file_name = dest_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| tracked.file_name.clone());

    if HistoryStore::normalize_path(&dest_str) != HistoryStore::normalize_path(&tracked.file_path) {
        let updated_path = dest_str.clone();
        run_blocking_store(store, move |s| {
            s.update_tracked_file_path(file_id, &updated_path)
        })
        .await?;
    }

    Ok(RecoverFileResult {
        file_path: dest_str,
        file_name: final_file_name,
        content,
    })
}

/// Returns the persisted version history settings.
#[tauri::command]
pub async fn history_get_settings(
    state: State<'_, HistoryState>,
) -> Result<HistorySettings, String> {
    let store = state.store()?;
    run_blocking_store(store, |s| s.get_settings()).await
}

/// Validates and updates version history settings, pruning snapshots as needed.
#[tauri::command]
pub async fn history_update_settings(
    state: State<'_, HistoryState>,
    settings: HistorySettings,
) -> Result<HistorySettings, String> {
    let store = state.store()?;
    run_blocking_store(store, move |s| s.update_settings(settings)).await
}

/// Lists snapshot summaries for the specified file path, newest first.
#[tauri::command]
pub async fn history_list_snapshots(
    state: State<'_, HistoryState>,
    file_path: String,
) -> Result<Vec<SnapshotSummary>, String> {
    let store = state.store()?;
    run_blocking_store(store, move |s| s.list_snapshots(&file_path)).await
}

/// Lists snapshot summaries for a tracked file ID, newest first.
#[tauri::command]
pub async fn history_list_snapshots_by_file_id(
    state: State<'_, HistoryState>,
    file_id: i64,
) -> Result<Vec<SnapshotSummary>, String> {
    let store = state.store()?;
    run_blocking_store(store, move |s| s.list_snapshots_by_file_id(file_id)).await
}

/// Reads and decompresses the full content of a snapshot.
#[tauri::command]
pub async fn history_get_snapshot_content(
    state: State<'_, HistoryState>,
    snapshot_id: i64,
) -> Result<String, String> {
    let store = state.store()?;
    run_blocking_store(store, move |s| s.get_snapshot_content(snapshot_id)).await
}

/// Computes a line-level diff between a stored snapshot and the provided current content.
#[tauri::command]
pub async fn history_diff_snapshot(
    state: State<'_, HistoryState>,
    snapshot_id: i64,
    current_content: String,
) -> Result<DiffResult, String> {
    let store = state.store()?;
    run_blocking_store(store, move |s| s.diff_snapshot(snapshot_id, &current_content)).await
}

/// Restores a snapshot to disk, saving current edits first and authorizing asset paths.
#[tauri::command]
pub async fn history_restore_snapshot(
    app: AppHandle,
    state: State<'_, HistoryState>,
    file_path: String,
    snapshot_id: i64,
    current_content: String,
) -> Result<String, String> {
    let store = state.store()?;
    let restored =
        restore_snapshot_inner(store, file_path.clone(), snapshot_id, current_content).await?;
    let path = Path::new(&file_path);
    register_asset_search_dir(path);
    allow_asset_dir(&app, path);
    Ok(restored)
}

/// Lists tracked files with snapshots that are currently missing from disk.
#[tauri::command]
pub async fn history_list_deleted_files(
    state: State<'_, HistoryState>,
) -> Result<Vec<DeletedFileSummary>, String> {
    let store = state.store()?;
    run_blocking_store(store, |s| s.list_deleted_files()).await
}

/// Recovers a deleted file from a snapshot and authorizes its destination for asset loading.
#[tauri::command]
pub async fn history_recover_file(
    app: AppHandle,
    state: State<'_, HistoryState>,
    file_id: i64,
    snapshot_id: i64,
    target_dir: Option<String>,
) -> Result<RecoverFileResult, String> {
    let store = state.store()?;
    let result = recover_file_inner(store, file_id, snapshot_id, target_dir).await?;
    let path = Path::new(&result.file_path);
    register_asset_search_dir(path);
    allow_asset_dir(&app, path);
    Ok(result)
}

/// Permanently deletes all version history for a tracked file ID.
#[tauri::command]
pub async fn history_delete_file_history(
    state: State<'_, HistoryState>,
    file_id: i64,
) -> Result<bool, String> {
    let store = state.store()?;
    run_blocking_store(store, move |s| {
        s.delete_file_history(file_id)?;
        Ok(true)
    })
    .await
}

/// Clears all tracked files and snapshots from the history store while keeping settings.
#[tauri::command]
pub async fn history_clear_all(state: State<'_, HistoryState>) -> Result<bool, String> {
    let store = state.store()?;
    run_blocking_store(store, |s| {
        s.clear_all()?;
        Ok(true)
    })
    .await
}

/// Returns aggregate counts and SQLite database size statistics.
#[tauri::command]
pub async fn history_get_stats(state: State<'_, HistoryState>) -> Result<HistoryStats, String> {
    let store = state.store()?;
    run_blocking_store(store, |s| s.get_stats()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_restore_snapshot_saves_pre_restore_and_writes_disk() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());

        let doc_path = dir.path().join("doc.md");
        fs::write(&doc_path, "v2").unwrap();
        let doc_str = doc_path.to_string_lossy().to_string();

        let snap_v1_id = store
            .record_snapshot_at(&doc_str, "v1", "initial", 1_000)
            .unwrap()
            .unwrap();
        store
            .record_snapshot_at(&doc_str, "v2", "manual", 2_000)
            .unwrap()
            .unwrap();

        let restored = restore_snapshot_inner(
            Arc::clone(&store),
            doc_str.clone(),
            snap_v1_id,
            "v3 unsaved edits".to_string(),
        )
        .await
        .unwrap();

        assert_eq!(restored, "v1");
        assert_eq!(fs::read_to_string(&doc_path).unwrap(), "v1");

        let snapshots = store.list_snapshots(&doc_str).unwrap();
        let pre_restore = snapshots
            .iter()
            .find(|s| s.trigger == "pre_restore")
            .expect("expected a pre_restore snapshot");
        assert_eq!(
            store.get_snapshot_content(pre_restore.id).unwrap(),
            "v3 unsaved edits"
        );

        let other_path = dir.path().join("other.md");
        let other_str = other_path.to_string_lossy().to_string();
        let count_before = store.list_snapshots(&other_str).unwrap().len();
        let mismatch_res = restore_snapshot_inner(
            Arc::clone(&store),
            other_str.clone(),
            snap_v1_id,
            "should not be saved".to_string(),
        )
        .await;
        assert!(mismatch_res.is_err());
        assert_eq!(
            store.list_snapshots(&other_str).unwrap().len(),
            count_before
        );
        assert!(!other_path.exists());
    }

    #[tokio::test]
    async fn test_recover_file_recreates_missing_parent_dir() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());

        let sub_dir = dir.path().join("sub");
        let nested_dir = sub_dir.join("nested");
        fs::create_dir_all(&nested_dir).unwrap();
        let lost_path = nested_dir.join("lost.md");
        fs::write(&lost_path, "lost content").unwrap();
        let lost_str = lost_path.to_string_lossy().to_string();

        let snap_id = store
            .record_snapshot(&lost_str, "lost content", "initial")
            .unwrap()
            .unwrap();
        let file_id = store.list_snapshots(&lost_str).unwrap()[0].file_id;

        fs::remove_dir_all(&sub_dir).unwrap();
        assert!(!lost_path.exists());
        assert_eq!(store.list_deleted_files().unwrap().len(), 1);

        let result = recover_file_inner(Arc::clone(&store), file_id, snap_id, None)
            .await
            .unwrap();

        assert_eq!(result.file_path, HistoryStore::normalize_path(&lost_str));
        assert_eq!(result.file_name, "lost.md");
        assert_eq!(result.content, "lost content");
        assert!(lost_path.exists());
        assert_eq!(fs::read_to_string(&lost_path).unwrap(), "lost content");
        assert!(store.list_deleted_files().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_recover_file_avoids_overwriting_existing_file_in_target_or_original() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());

        let orig_dir = dir.path().join("orig");
        let other_dir = dir.path().join("other");
        fs::create_dir_all(&orig_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();

        let note_path = orig_dir.join("note.md");
        let note_str = note_path.to_string_lossy().to_string();
        let snap_id = store
            .record_snapshot(&note_str, "old", "initial")
            .unwrap()
            .unwrap();
        let file_id = store.list_snapshots(&note_str).unwrap()[0].file_id;

        fs::write(&note_path, "brand new").unwrap();
        let other_note_path = other_dir.join("note.md");
        fs::write(&other_note_path, "existing").unwrap();

        let res_other = recover_file_inner(
            Arc::clone(&store),
            file_id,
            snap_id,
            Some(other_dir.to_string_lossy().to_string()),
        )
        .await
        .unwrap();

        let expected_other_path = other_dir.join("note 2.md");
        assert_eq!(
            res_other.file_path,
            expected_other_path.to_string_lossy().to_string()
        );
        assert_eq!(res_other.file_name, "note 2.md");
        assert_eq!(res_other.content, "old");
        assert_eq!(fs::read_to_string(&expected_other_path).unwrap(), "old");
        assert_eq!(fs::read_to_string(&other_note_path).unwrap(), "existing");
        assert_eq!(fs::read_to_string(&note_path).unwrap(), "brand new");
        let updated_record = store.get_tracked_file(file_id).unwrap();
        assert_eq!(
            updated_record.file_path,
            HistoryStore::normalize_path(&expected_other_path.to_string_lossy())
        );
        assert_eq!(updated_record.file_name, "note 2.md");

        let orig2_dir = dir.path().join("orig2");
        fs::create_dir_all(&orig2_dir).unwrap();
        let item_path = orig2_dir.join("item.md");
        let item_str = item_path.to_string_lossy().to_string();
        let item_snap_id = store
            .record_snapshot(&item_str, "old item", "initial")
            .unwrap()
            .unwrap();
        let item_file_id = store.list_snapshots(&item_str).unwrap()[0].file_id;

        fs::write(&item_path, "brand new item").unwrap();

        let res_orig = recover_file_inner(Arc::clone(&store), item_file_id, item_snap_id, None)
            .await
            .unwrap();

        let expected_orig_path = orig2_dir.join("item 2.md");
        assert_eq!(
            res_orig.file_path,
            expected_orig_path.to_string_lossy().to_string()
        );
        assert_eq!(res_orig.file_name, "item 2.md");
        assert_eq!(res_orig.content, "old item");
        assert_eq!(fs::read_to_string(&expected_orig_path).unwrap(), "old item");
        assert_eq!(fs::read_to_string(&item_path).unwrap(), "brand new item");
        let updated_item = store.get_tracked_file(item_file_id).unwrap();
        assert_eq!(
            updated_item.file_path,
            HistoryStore::normalize_path(&expected_orig_path.to_string_lossy())
        );
        assert_eq!(updated_item.file_name, "item 2.md");
    }

    #[tokio::test]
    async fn test_recover_file_rejects_malicious_file_name() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let dir = tempdir().unwrap();
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path.clone()).unwrap());

        let target_dir = dir.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"escape content").unwrap();
        let compressed = encoder.finish().unwrap();

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let fake_path = dir.path().join("orig").join("escape.md");
        conn.execute(
            "INSERT INTO tracked_files (file_path, file_name, created_at, updated_at)
             VALUES (?1, ?2, 1000, 1000)",
            rusqlite::params![fake_path.to_string_lossy().to_string(), "../escape.md"],
        )
        .unwrap();
        let file_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO snapshots (
                file_id, created_at, trigger, content_hash, content_gz,
                size_bytes, compressed_bytes, line_count, word_count
             ) VALUES (?1, 1000, 'manual', 'deadbeef', ?2, 14, ?3, 1, 2)",
            rusqlite::params![file_id, compressed, compressed.len() as i64],
        )
        .unwrap();
        let snap_id = conn.last_insert_rowid();
        drop(conn);

        let res = recover_file_inner(
            Arc::clone(&store),
            file_id,
            snap_id,
            Some(target_dir.to_string_lossy().to_string()),
        )
        .await;

        assert!(res.is_err());
        assert!(!dir.path().join("escape.md").exists());
        assert!(!target_dir.join("escape.md").exists());
    }
}
