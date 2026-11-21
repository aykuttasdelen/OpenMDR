# Local Version History & File Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build Phase 1 local per-file version history, line-based diffing, in-place/copy restoration, and deleted-file recovery backed by an isolated SQLite database in OpenMD.

**Architecture:** A standalone Rust `HistoryStore` module manages an isolated SQLite database (`history.db` in `app_local_data_dir()`, WAL mode) storing gzip-compressed UTF-8 snapshots, SHA-256 hashes, tracked file paths, and settings. Thin async Tauri IPC commands (`history_ops.rs`) and hooks in `fs_ops.rs`/`dialogs.rs` call `HistoryStore` via `tokio::task::spawn_blocking` so file saves are never blocked or failed by history errors. A React `HistoryModal` (Current File diff/preview + Deleted Files recovery) and a new section in `SettingsModal` (`files` tab) expose the full workflow in Turkish and English.

**Tech Stack:** Rust (edition 2024, MSRV 1.90), Tauri v2, `rusqlite` (`bundled`), `flate2`, `sha2`, `similar`, React 19, TypeScript 5.8, Vite 6, Lucide icons, Node native test runner (`node --experimental-strip-types`).

**Spec:** `docs/superpowers/specs/2026-11-21-local-version-history-design.md`

## Global Constraints

- Database path: `app.path().app_local_data_dir()?.join("history.db")` (macOS: `~/Library/Application Support/com.openmd.editor/history.db`), outside user workspaces.
- SQLite pragmas on open: `PRAGMA journal_mode = WAL;`, `PRAGMA foreign_keys = ON;`, `PRAGMA busy_timeout = 5000;`, `PRAGMA user_version = 1;`.
- All SQLite and diff operations run inside `tokio::task::spawn_blocking`; never block the Tauri async executor or UI thread.
- Best-effort recording: SQLite/history failures MUST NEVER fail or block `fs_ops::write_file` or `dialogs::save_file_dialog` (log via `eprintln!` and continue).
- Settings source of truth: `history.db` `settings` table with defaults `enabled = true`, `max_versions_per_file = 50` (valid `5..=200`), `autosave_interval_minutes = 3` (valid `1..=60`). Decreasing `max_versions_per_file` prunes excess snapshots immediately.
- Snapshot cap: skip snapshots whose uncompressed UTF-8 size exceeds `10 * 1024 * 1024` bytes (10 MB).
- Diff cap: truncate diff line output above `50_000` lines with `truncated: true`.
- Bilingual parity: every key added to `src/i18n/translations.ts` MUST exist non-empty in both `tr` and `en` (enforced by `npm test`).
- JSON field naming: Rust structs use `#[serde(rename_all = "camelCase")]`; Tauri commands accept camelCase from JS automatically.

## Review Focus

1. **Oversized (>10 MB) or non-UTF-8 on-disk files during `initial` baseline capture:** Opening/saving a file larger than 10 MB or containing invalid UTF-8 on disk must not fail `write_file` and must skip creating an invalid/oversized snapshot; modifying a normal pre-existing UTF-8 file for the first time must capture both `'initial'` (old disk content) and `'manual'` (new content) in chronological order. *(Pinned in Task 1 & Task 3)*
2. **Rename collision merge & directory prefix boundary:** Renaming `a.md` onto `b.md` when `b.md` already exists in `tracked_files` as a deleted entry must merge snapshots without `UNIQUE` constraint errors and prune to `max_versions_per_file`; renaming directory `/docs` -> `/notes` must update `/docs/x.md` without modifying sibling `/docs2/x.md`. *(Pinned in Task 1)*
3. **`pre_restore` must bypass autosave throttling and deduplicate:** Restoring an older snapshot immediately after an autosave (inside the 3-minute window) must still capture unsaved editor edits as `'pre_restore'`, while restoring when `current_content` already matches the latest snapshot must not insert a duplicate. *(Pinned in Task 1 & Task 2)*
4. **Recovery safety (missing parent dirs, existing file collision, path traversal):** Recovering a deleted file whose parent folder was deleted must recreate parent directories; if a file already exists at the target path, recovery must generate a collision-free name via `resolve_available_path` rather than overwriting; a malicious `file_name` (e.g. `../escape.md`) must be rejected via `validate_single_component`. *(Pinned in Task 2)*
5. **Best-effort write durability & large-file diff truncation:** Disabling history or encountering a storage error during `write_file` must still succeed in writing the target file; diffing two >50,000-line documents must return `truncated: true` without hanging. *(Pinned in Task 1 & Task 3)*

---

## File Structure

**Created:**
- `src-tauri/src/history_store.rs` — Pure Rust SQLite storage, compression, hybrid snapshot policy, rename tracking, deleted listing, stats, and `similar` diff engine + unit tests.
- `src-tauri/src/commands/history_ops.rs` — 12 async Tauri IPC commands wrapping `HistoryStore` via `spawn_blocking` + recovery/restore integration tests.
- `src/utils/historyUtils.ts` — Pure TS helpers (`formatRelativeTimestamp`, `formatByteSize`, `buildCopyTabTitle`, `clampHistorySettings`).
- `src/components/HistoryModal.tsx` — 2-tab modal (`Bu Dosya` diff/content viewer + `Silinen / Kayıp Dosyalar` recovery browser).

**Modified:**
- `src-tauri/Cargo.toml` — Add `rusqlite` (`bundled`), `flate2`, `sha2`, `similar`.
- `src-tauri/src/commands/mod.rs` — Export `history_ops`.
- `src-tauri/src/commands/fs_ops.rs` — Expose `pub(crate) fn validate_single_component` & `pub(crate) fn resolve_available_path`; hook `write_file` (`save_source`) and `rename_file` into `HistoryStore`.
- `src-tauri/src/commands/dialogs.rs` — Hook `save_file_dialog` into `HistoryStore`.
- `src-tauri/src/menu.rs` — Add `version-history` (`CmdOrCtrl+Shift+H`) item to File menu.
- `src-tauri/src/lib.rs` — Wire `history_store`, initialize `HistoryStore` in `AppState`, register all 12 `history_ops` commands.
- `src/types/electron.d.ts` — History DTO interfaces + `ElectronAPI` methods + optional `saveSource` on `writeFile`.
- `src/api/tauriBridge.ts` — IPC calls for `writeFile(..., saveSource?)` and all 12 history commands.
- `src/i18n/translations.ts` — TR & EN strings for history modal, settings, titlebar, command palette.
- `src/index.css` — Dark/light diff palette variables and `HistoryModal` layout styles.
- `src/components/SettingsModal.tsx` — Version history configuration & storage stats card in `files` tab.
- `src/components/TitleBar.tsx` — History icon button (`onOpenHistory`).
- `src/App.tsx` — Connect history settings/stats, explicit `saveSource` tags, `HistoryModal`, `TitleBar`, `CommandPalette`, and menu/keyboard triggers.
- `test/test-utils.js` — Unit tests for `historyUtils.ts` and new i18n keys.

---

### Task 1: Rust Core Storage & Diff Engine (`history_store.rs`)

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/history_store.rs`
- Modify: `src-tauri/src/lib.rs:1-4` (add `pub mod history_store;`)

**Interfaces:**
- Consumes: `rusqlite`, `flate2`, `sha2`, `similar`, `serde`.
- Produces (all structs `#[derive(Debug, Clone, Serialize, Deserialize)]` with `#[serde(rename_all = "camelCase")]`):
  - Constants: `DEFAULT_MAX_VERSIONS: u32 = 50`, `MIN_MAX_VERSIONS: u32 = 5`, `MAX_MAX_VERSIONS: u32 = 200`, `DEFAULT_AUTOSAVE_INTERVAL_MINUTES: u32 = 3`, `MIN_AUTOSAVE_INTERVAL_MINUTES: u32 = 1`, `MAX_AUTOSAVE_INTERVAL_MINUTES: u32 = 60`, `MAX_SNAPSHOT_BYTES: usize = 10 * 1024 * 1024`, `MAX_DIFF_LINES: usize = 50_000`.
  - `pub struct HistorySettings { pub enabled: bool, pub max_versions_per_file: u32, pub autosave_interval_minutes: u32 }`
  - `pub struct SnapshotSummary { pub id: i64, pub file_id: i64, pub created_at: i64, pub trigger: String, pub size_bytes: i64, pub compressed_bytes: i64, pub line_count: i64, pub word_count: i64 }`
  - `pub struct DeletedFileSummary { pub file_id: i64, pub file_path: String, pub file_name: String, pub updated_at: i64, pub snapshot_count: i64, pub latest_snapshot_id: i64, pub latest_size_bytes: i64 }`
  - `pub struct DiffLine { pub tag: String, pub old_line_no: Option<usize>, pub new_line_no: Option<usize>, pub content: String }`
  - `pub struct DiffResult { pub lines: Vec<DiffLine>, pub added_count: usize, pub removed_count: usize, pub unchanged_count: usize, pub truncated: bool }`
  - `pub struct HistoryStats { pub total_files: i64, pub total_snapshots: i64, pub db_size_bytes: u64 }`
  - `pub struct TrackedFileRecord { pub id: i64, pub file_path: String, pub file_name: String }`
  - `pub struct HistoryStore` with methods:
    - `pub fn open(db_path: PathBuf) -> Result<Self, String>`
    - `pub fn normalize_path(path: &str) -> String`
    - `pub fn get_settings(&self) -> Result<HistorySettings, String>`
    - `pub fn update_settings(&self, settings: HistorySettings) -> Result<HistorySettings, String>`
    - `pub fn has_snapshots(&self, file_path: &str) -> Result<bool, String>`
    - `pub fn record_snapshot(&self, file_path: &str, content: &str, trigger: &str) -> Result<Option<i64>, String>`
    - `pub fn record_snapshot_at(&self, file_path: &str, content: &str, trigger: &str, now_ms: i64) -> Result<Option<i64>, String>` (deterministic timestamp helper for tests & internal use)
    - `pub fn list_snapshots(&self, file_path: &str) -> Result<Vec<SnapshotSummary>, String>`
    - `pub fn list_snapshots_by_file_id(&self, file_id: i64) -> Result<Vec<SnapshotSummary>, String>`
    - `pub fn get_snapshot_content(&self, snapshot_id: i64) -> Result<String, String>`
    - `pub fn get_tracked_file(&self, file_id: i64) -> Result<TrackedFileRecord, String>`
    - `pub fn diff_snapshot(&self, snapshot_id: i64, current_content: &str) -> Result<DiffResult, String>`
    - `pub fn handle_rename(&self, old_path: &str, new_path: &str) -> Result<(), String>`
    - `pub fn update_tracked_file_path(&self, file_id: i64, new_path: &str) -> Result<(), String>`
    - `pub fn list_deleted_files(&self) -> Result<Vec<DeletedFileSummary>, String>`
    - `pub fn delete_file_history(&self, file_id: i64) -> Result<(), String>`
    - `pub fn clear_all(&self) -> Result<(), String>`
    - `pub fn get_stats(&self) -> Result<HistoryStats, String>`

- [ ] **Step 1: Add Rust dependencies to `src-tauri/Cargo.toml` and declare module in `src-tauri/src/lib.rs`**

Add under `[dependencies]` in `src-tauri/Cargo.toml`:
```toml
rusqlite = { version = "0.35", features = ["bundled"] }
flate2 = "1.1"
sha2 = "0.10"
similar = "2.7"
```
Add `pub mod history_store;` at top of `src-tauri/src/lib.rs`.

- [ ] **Step 2: Write failing unit tests in `src-tauri/src/history_store.rs`**

Create `src-tauri/src/history_store.rs` with stub structs/methods returning `Err("todo".into())` and a `#[cfg(test)] mod tests` suite containing these exact tests:
1. `test_schema_and_default_settings`: opens DB in `tempdir()`, asserts default `HistorySettings { enabled: true, max_versions_per_file: 50, autosave_interval_minutes: 3 }`, verifies `PRAGMA user_version == 1` and `journal_mode == "wal"`.
2. `test_settings_validation_and_immediate_prune`: inserts 12 manual snapshots for `/tmp/a.md`, calls `update_settings` with `max_versions_per_file: 5`, asserts `list_snapshots` returns 5 newest; asserts `max_versions_per_file: 4` and `201` and `autosave_interval_minutes: 0` and `61` return `Err`.
3. `test_hybrid_throttling_dedup_and_pre_restore`:
   - `record_snapshot_at("/tmp/doc.md", "v1", "initial", 1_000)` -> `Some(_)`
   - `record_snapshot_at("/tmp/doc.md", "v1", "manual", 2_000)` -> `None` (SHA-256 dedup)
   - `record_snapshot_at("/tmp/doc.md", "v2", "autosave", 60_000)` -> `None` (within 3 min = 180_000 ms window)
   - `record_snapshot_at("/tmp/doc.md", "v2", "autosave", 181_000)` -> `Some(_)` (past 3 min window)
   - `record_snapshot_at("/tmp/doc.md", "v3", "manual", 182_000)` -> `Some(_)` (manual ignores throttle)
   - `record_snapshot_at("/tmp/doc.md", "v4 unsaved", "pre_restore", 183_000)` -> `Some(_)` (`pre_restore` ignores throttle)
   - `record_snapshot_at("/tmp/doc.md", "v4 unsaved", "pre_restore", 184_000)` -> `None` (`pre_restore` still deduplicates)
4. `test_oversized_snapshot_skipped_and_disabled_setting`:
   - content of `MAX_SNAPSHOT_BYTES + 1` bytes returns `Ok(None)` and creates 0 snapshots.
   - setting `enabled: false` causes `record_snapshot_at` for `"manual"` / `"autosave"` to return `Ok(None)`.
5. `test_rename_file_collision_merge_and_directory_prefix_boundary`:
   - records snapshots for `/w/a.md` (3 versions) and `/w/b.md` (4 versions), sets limit to `5`, calls `handle_rename("/w/a.md", "/w/b.md")` -> `/w/a.md` has 0 snapshots, `/w/b.md` has 5 newest merged snapshots.
   - records snapshots for `/w/docs/n1.md` and `/w/docs2/n2.md`, calls `handle_rename("/w/docs", "/w/notes")` -> `/w/notes/n1.md` has snapshot, `/w/docs/n1.md` is empty, `/w/docs2/n2.md` is untouched!
6. `test_deleted_files_listing_and_clear_all`:
   - creates real file `dir.path().join("keep.md")` and deletes `dir.path().join("gone.md")` after recording both; `list_deleted_files()` returns only `gone.md` with `snapshot_count == 1`.
   - `get_stats()` reports `total_files == 2`, `total_snapshots == 2`, `db_size_bytes > 0`.
   - `clear_all()` leaves `total_files == 0`, `total_snapshots == 0`.
7. `test_diff_snapshot_and_truncation`:
   - records `"line1\nline2\nline3\n"`, diffs against `"line1\nline2-mod\nline3\nline4\n"`, asserts `added_count == 2`, `removed_count == 1`, `unchanged_count == 2`, `truncated == false`, and line tags/numbers match.
   - builds a 50,050-line snapshot and diffs against a modified copy, asserts `truncated == true` and `lines.len() <= MAX_DIFF_LINES`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml history_store`
Expected: FAIL (stub methods return `Err("todo")`).

- [ ] **Step 4: Implement `HistoryStore` in `src-tauri/src/history_store.rs`**

Implement all methods per spec §4.1–§4.5:
- Wrap `Mutex<Connection>` + `db_path: PathBuf` inside `HistoryStore`.
- Apply `WAL`, `foreign_keys = ON`, `busy_timeout = 5000`, `user_version = 1`, and DDL from spec §4.2 in `open()`.
- `normalize_path`: trim trailing `/` (and `\` on Windows) except root `/`.
- Compress content with `flate2::write::GzEncoder` (`Compression::default()`) and hash uncompressed UTF-8 bytes with `sha2::Sha256` (lowercase hex).
- Enforce `MAX_SNAPSHOT_BYTES` (`10 * 1024 * 1024`), dedup against latest `content_hash`, throttle ONLY when `trigger == "autosave"`, and prune per-file snapshots ordered by `created_at DESC, id DESC` beyond `max_versions_per_file` inside a single SQLite transaction.
- In `handle_rename`, handle both exact file path rename (merging snapshots if target `file_path` already exists in `tracked_files`, then pruning to limit) and directory rename (strict prefix `old_norm + "/"` or `old_norm + "\\"`, avoiding false matches like `/docs2`).
- In `diff_snapshot`, use `similar::TextDiff::from_lines(&old_content, current_content)`, tracking 1-based `old_line_no` and `new_line_no`, capping emitted `lines` at `MAX_DIFF_LINES` (`50_000`) and setting `truncated = true` when exceeded.
- In `clear_all`, delete from `tracked_files` (cascades to `snapshots`) and execute `VACUUM`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml history_store`
Expected: PASS (all 7 tests pass with 0 failures).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/history_store.rs src-tauri/src/lib.rs
git commit -m "feat(history): add SQLite HistoryStore and diff engine"
```

---

### Task 2: Tauri IPC Commands (`history_ops.rs`) & AppState Registration

**Files:**
- Modify: `src-tauri/src/commands/fs_ops.rs:7-27,262-308` (expose `pub(crate)` helpers `validate_single_component` and `resolve_available_path`)
- Create: `src-tauri/src/commands/history_ops.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs:1-178`

**Interfaces:**
- Consumes: `crate::history_store::*`, `crate::commands::fs_ops::{validate_single_component, resolve_available_path}`, `crate::commands::image_ops::{register_asset_search_dir, allow_asset_dir}`.
- Produces:
  - `AppState` field: `pub history: Option<Arc<HistoryStore>>` (graceful `Option` so if app_local_data_dir fails to open DB on startup, app still launches cleanly instead of panicking!).
  - `pub struct RecoverFileResult { pub file_path: String, pub file_name: String, pub content: String }` (`#[serde(rename_all = "camelCase")]`).
  - Core testable async functions (called by the Tauri commands):
    - `pub async fn restore_snapshot_inner(store: Arc<HistoryStore>, file_path: String, snapshot_id: i64, current_content: String) -> Result<String, String>`
    - `pub async fn recover_file_inner(store: Arc<HistoryStore>, file_id: i64, snapshot_id: i64, target_dir: Option<String>) -> Result<RecoverFileResult, String>`
  - All 12 `#[tauri::command]` functions in `history_ops.rs`:
    1. `history_get_settings(state)` -> `Result<HistorySettings, String>`
    2. `history_update_settings(state, settings: HistorySettings)` -> `Result<HistorySettings, String>`
    3. `history_list_snapshots(state, file_path: String)` -> `Result<Vec<SnapshotSummary>, String>`
    4. `history_list_snapshots_by_file_id(state, file_id: i64)` -> `Result<Vec<SnapshotSummary>, String>`
    5. `history_get_snapshot_content(state, snapshot_id: i64)` -> `Result<String, String>`
    6. `history_diff_snapshot(state, snapshot_id: i64, current_content: String)` -> `Result<DiffResult, String>`
    7. `history_restore_snapshot(app, state, file_path: String, snapshot_id: i64, current_content: String)` -> `Result<String, String>`
    8. `history_list_deleted_files(state)` -> `Result<Vec<DeletedFileSummary>, String>`
    9. `history_recover_file(app, state, file_id: i64, snapshot_id: i64, target_dir: Option<String>)` -> `Result<RecoverFileResult, String>`
    10. `history_delete_file_history(state, file_id: i64)` -> `Result<bool, String>`
    11. `history_clear_all(state)` -> `Result<bool, String>`
    12. `history_get_stats(state)` -> `Result<HistoryStats, String>`

- [ ] **Step 1: Expose `validate_single_component` and `resolve_available_path` in `fs_ops.rs`**

Change `fn validate_single_component` to `pub(crate) fn validate_single_component` and extract the body of `get_available_path` into `pub(crate) fn resolve_available_path(base_dir: &Path, file_name: &str) -> Result<PathBuf, String>` (called by `get_available_path`).

- [ ] **Step 2: Write failing async tests in `src-tauri/src/commands/history_ops.rs`**

Add `pub mod history_ops;` in `src-tauri/src/commands/mod.rs`. Create `src-tauri/src/commands/history_ops.rs` with stub `restore_snapshot_inner` / `recover_file_inner` and tests:
1. `test_restore_snapshot_saves_pre_restore_and_writes_disk`:
   - writes `v1` and `v2` snapshots for `doc.md`, calls `restore_snapshot_inner(store, path, snap_v1_id, "v3 unsaved edits")`.
   - asserts returned content is `"v1"`, disk file contains `"v1"`, and `list_snapshots` includes a `"pre_restore"` snapshot whose content is `"v3 unsaved edits"`.
2. `test_recover_file_recreates_missing_parent_dir`:
   - records snapshot for `tmp/sub/nested/lost.md`, removes `tmp/sub`, calls `recover_file_inner(store, file_id, snap_id, None)`.
   - asserts `tmp/sub/nested/lost.md` exists on disk with snapshot content and `list_deleted_files()` is now empty.
3. `test_recover_file_avoids_overwriting_existing_file_in_target_or_original`:
   - records snapshot for `tmp/orig/note.md` (`"old"`), creates a new file at `tmp/orig/note.md` (`"brand new"`) and at `tmp/other/note.md` (`"existing"`).
   - recovers to `Some(tmp/other)` -> writes `tmp/other/note 2.md` with `"old"`, leaves `tmp/other/note.md` intact (`"existing"`), and updates `tracked_files.file_path` to `tmp/other/note 2.md`.
4. `test_recover_file_rejects_malicious_file_name`:
   - inserts a `tracked_files` row with `file_name = "../escape.md"` (simulating corrupted DB row), calls `recover_file_inner(..., Some(target_dir))`, asserts `Err(...)` and no file created outside `target_dir`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml history_ops`
Expected: FAIL.

- [ ] **Step 4: Implement `history_ops.rs` and wire `AppState` + `invoke_handler!` in `src-tauri/src/lib.rs`**

- Implement all 12 commands delegating blocking SQLite/diff calls to `tokio::task::spawn_blocking`.
- In `restore_snapshot_inner`: (1) verify `snapshot_id` belongs to `file_path` (or fetch target content), (2) call `store.record_snapshot(&file_path, &current_content, "pre_restore")` BEFORE writing disk, (3) create parent dirs if needed & `tokio::fs::write(&file_path, &target_content)`, (4) return `target_content`.
- In `recover_file_inner`: fetch `TrackedFileRecord` and snapshot content; validate `file_name` via `validate_single_component`; if `target_dir` is `None`, check parent dir (`create_dir_all`) and use `resolve_available_path(parent, file_name)`; if `Some(dir)`, use `resolve_available_path(Path::new(&dir), file_name)`; write file; update tracked path if changed; return `RecoverFileResult`.
- In `history_restore_snapshot` and `history_recover_file` commands, also call `image_ops::register_asset_search_dir` and `image_ops::allow_asset_dir` for the restored path.
- In `src-tauri/src/lib.rs`: open `HistoryStore` inside `setup()` using `app.path().app_local_data_dir()?.join("history.db")`, store in `AppState` (e.g. `Arc<std::sync::OnceLock<Arc<HistoryStore>>>` or initialize via `app.manage(HistoryState(Option<Arc<HistoryStore>>))` inside `setup()` so `AppState` stays simple and commands take `State<'_, HistoryState>`). Register all 12 commands in `invoke_handler!`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml history_ops`
Expected: PASS (all 4 tests pass).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/fs_ops.rs src-tauri/src/commands/history_ops.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(history): add Tauri IPC commands and AppState wiring"
```

---

### Task 3: Backend File Lifecycle Hooks (`write_file`, `save_file_dialog`, `rename_file`) & Native Menu

**Files:**
- Modify: `src-tauri/src/commands/fs_ops.rs:144-166,243-250`
- Modify: `src-tauri/src/commands/dialogs.rs:52-84`
- Modify: `src-tauri/src/menu.rs:24-44`
- Modify: `src-tauri/src/lib.rs:180-442` (add integration tests for hooks)

**Interfaces:**
- Consumes: `HistoryState`, `HistoryStore::has_snapshots`, `HistoryStore::record_snapshot`, `HistoryStore::handle_rename`, `MAX_SNAPSHOT_BYTES`.
- Produces:
  - Helper in `history_ops.rs`: `pub(crate) async fn record_write_with_baseline(store: Option<Arc<HistoryStore>>, file_path: String, pre_existing_content: Option<String>, new_content: String, save_source: String)` and `pub(crate) async fn capture_pre_write_baseline(store: Option<Arc<HistoryStore>>, file_path: &Path) -> Option<String>`.
  - Updated Tauri command signature: `fs_ops::write_file(app, history: State<'_, HistoryState>, file_path: String, content: String, save_source: Option<String>) -> Result<bool, String>`.
  - Updated `dialogs::save_file_dialog(history: State<'_, HistoryState>, default_name, default_dir, content)`.
  - Updated `fs_ops::rename_file(history: State<'_, HistoryState>, old_path: String, new_path: String) -> Result<bool, String>`.
  - Native menu item ID `"version-history"` in `menu.rs` under File menu (`"Sürüm Geçmişi..."` / `"Version History..."`, shortcut `Some("CmdOrCtrl+Shift+H")`).

- [ ] **Step 1: Write failing integration tests in `src-tauri/src/lib.rs` (or `history_ops.rs`)**

Add tests for the exact lifecycle helper functions:
1. `test_write_hook_captures_initial_baseline_then_manual_and_skips_non_utf8`:
   - Case A (normal pre-existing UTF-8 file): creates `existing.md` with `"original disk text"`, runs `capture_pre_write_baseline` -> writes `"edited text"` -> runs `record_write_with_baseline(..., "manual")`. Asserts 2 snapshots exist: older has trigger `"initial"` and content `"original disk text"`, newer has trigger `"manual"` and content `"edited text"`.
   - Case B (invalid UTF-8 on disk): creates `binary.md` with bytes `[0xff, 0xfe, 0x00]`, runs `capture_pre_write_baseline` -> returns `None` without error; subsequent `record_write_with_baseline` records only `"manual"` snapshot cleanly.
   - Case C (`store == None`): runs `capture_pre_write_baseline(None, ...)` and `record_write_with_baseline(None, ...)` -> completes cleanly with no panic or error.
2. `test_write_hook_autosave_throttles_without_losing_disk_write`:
   - verifies `save_source = "autosave"` respects `autosave_interval_minutes` while disk write still completes.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml test_write_hook`
Expected: FAIL (helpers not defined yet).

- [ ] **Step 3: Implement baseline capture & post-write hooks and wire `fs_ops.rs`, `dialogs.rs`, `menu.rs`**

- In `capture_pre_write_baseline`: if `store` is `Some(s)`, check `s.get_settings()?.enabled` and `!s.has_snapshots(path)` and file exists with `metadata.len() <= MAX_SNAPSHOT_BYTES as u64`; read bytes via `tokio::fs::read` and convert with `String::from_utf8(bytes).ok()`. Any error logs to stderr and returns `None`.
- In `record_write_with_baseline`: in `spawn_blocking`, if `pre_existing_content` is `Some(old)` and `old != new_content`, call `store.record_snapshot(&file_path, &old, "initial")` (log errors, never fail); then normalize `save_source` (`"autosave"` | `"initial"` | default `"manual"`) and call `store.record_snapshot(&file_path, &new_content, trigger)` (log errors, never fail).
- Wire `fs_ops::write_file`: capture baseline before `tokio::fs::write`, then call `record_write_with_baseline` after `tokio::fs::write` succeeds.
- Wire `dialogs::save_file_dialog`: when `content` is `Some(text)`, capture baseline before `tokio::fs::write`, then call `record_write_with_baseline(..., "manual")` after write succeeds.
- Wire `fs_ops::rename_file`: after `tokio::fs::rename` succeeds, if `store` is `Some(s)`, run `s.handle_rename(&old_path, &new_path)` in `spawn_blocking` (log errors, never fail rename).
- Wire `menu.rs`: add `&MenuItem::with_id(app, "version-history", if is_english { "Version History..." } else { "Sürüm Geçmişi..." }, true, Some("CmdOrCtrl+Shift+H"))?` in `file_menu` right after `save-file-as` and a separator.

- [ ] **Step 4: Run full Rust test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all existing + new Rust tests pass, 0 warnings/errors).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/history_ops.rs src-tauri/src/commands/fs_ops.rs src-tauri/src/commands/dialogs.rs src-tauri/src/menu.rs src-tauri/src/lib.rs
git commit -m "feat(history): hook file writes, save dialog, rename, and native menu"
```

---

### Task 4: Frontend Bridge, Types, Pure Utilities (`historyUtils.ts`) & Bilingual i18n

**Files:**
- Create: `src/utils/historyUtils.ts`
- Modify: `src/types/electron.d.ts`
- Modify: `src/api/tauriBridge.ts`
- Modify: `src/i18n/translations.ts`
- Modify: `test/test-utils.js`

**Interfaces:**
- Consumes: Tauri IPC commands from Task 2 & Task 3.
- Produces:
  - In `src/utils/historyUtils.ts`:
    - `export function formatByteSize(bytes: number): string` (e.g. `0 B`, `512 B`, `1.5 KB`, `2.3 MB`)
    - `export function formatRelativeTimestamp(epochMs: number, lang: 'tr' | 'en', nowMs?: number): string` (`Az önce` / `Just now` <60s; `{n} dk önce` / `{n} min ago` <60m; `{n} sa önce` / `{n}h ago` <24h; localized date+time otherwise)
    - `export function buildCopyTabTitle(fileName: string, createdAtMs: number, lang: 'tr' | 'en'): string` (e.g. `notlar (14-32 kopya).md` / `notes (14-32 copy).md`, preserving `.md`/`.mdx` extension)
    - `export function clampHistorySettings(input: { enabled: boolean; maxVersionsPerFile: number; autosaveIntervalMinutes: number }): { enabled: boolean; maxVersionsPerFile: number; autosaveIntervalMinutes: number }` (clamps `maxVersionsPerFile` to `5..200` and `autosaveIntervalMinutes` to `1..60`, rounding integers)
  - In `src/types/electron.d.ts`:
    - `export type SaveSource = 'manual' | 'autosave' | 'initial';`
    - `export type HistoryTrigger = 'initial' | 'manual' | 'autosave' | 'pre_restore';`
    - `export interface HistorySettings { enabled: boolean; maxVersionsPerFile: number; autosaveIntervalMinutes: number; }`
    - `export interface SnapshotSummary { id: number; fileId: number; createdAt: number; trigger: HistoryTrigger; sizeBytes: number; compressedBytes: number; lineCount: number; wordCount: number; }`
    - `export interface DeletedFileSummary { fileId: number; filePath: string; fileName: string; updatedAt: number; snapshotCount: number; latestSnapshotId: number; latestSizeBytes: number; }`
    - `export interface DiffLine { tag: 'equal' | 'insert' | 'delete'; oldLineNo: number | null; newLineNo: number | null; content: string; }`
    - `export interface DiffResult { lines: DiffLine[]; addedCount: number; removedCount: number; unchangedCount: number; truncated: boolean; }`
    - `export interface HistoryStats { totalFiles: number; totalSnapshots: number; dbSizeBytes: number; }`
    - `export interface RecoverFileResult { filePath: string; fileName: string; content: string; }`
    - `ElectronAPI`: update `writeFile: (filePath: string, content: string, saveSource?: SaveSource) => Promise<boolean>;` and add optional-free methods: `historyGetSettings`, `historyUpdateSettings`, `historyListSnapshots`, `historyListSnapshotsByFileId`, `historyGetSnapshotContent`, `historyDiffSnapshot`, `historyRestoreSnapshot`, `historyListDeletedFiles`, `historyRecoverFile`, `historyDeleteFileHistory`, `historyClearAll`, `historyGetStats`.
  - In `src/i18n/translations.ts`: matching TR & EN keys for `titlebar.history`, `command.versionHistory`, `command.recoverDeleted`, `settings.historyTitle`, `settings.historyEnabled`, `settings.historyEnabledDesc`, `settings.historyMaxVersions`, `settings.historyMaxVersionsDesc`, `settings.historyAutosaveInterval`, `settings.historyAutosaveIntervalDesc`, `settings.historyMinutes`, `settings.historyStorageStats`, `settings.historyClearAll`, `settings.historyClearAllConfirm`, `settings.historyCleared`, `history.*` (tabs, triggers, diff/content toggle, stats, empty states, buttons, confirmations, truncation notice, errors).

- [ ] **Step 1: Write failing tests in `test/test-utils.js`**

Add Test 19 (`historyUtils` tests) and expand Test 12 (i18n checks) in `test/test-utils.js`:
- Import `formatByteSize`, `formatRelativeTimestamp`, `buildCopyTabTitle`, `clampHistorySettings` from `../src/utils/historyUtils.ts`.
- Assert `formatByteSize(0) === '0 B'`, `formatByteSize(1536) === '1.5 KB'`, `formatByteSize(2 * 1024 * 1024) === '2.0 MB'`.
- Assert `formatRelativeTimestamp(10_000, 'tr', 40_000) === 'Az önce'`, `formatRelativeTimestamp(10_000, 'en', 40_000) === 'Just now'`, `formatRelativeTimestamp(0, 'tr', 5 * 60_000) === '5 dk önce'`, `formatRelativeTimestamp(0, 'en', 2 * 3600_000) === '2h ago'`.
- Assert `buildCopyTabTitle('notes.md', new Date(2026, 10, 21, 14, 32).getTime(), 'tr') === 'notes (14-32 kopya).md'` and `'en'` ends with `(14-32 copy).md`, and `'guide.mdx'` preserves `.mdx`.
- Assert `clampHistorySettings({ enabled: true, maxVersionsPerFile: 1, autosaveIntervalMinutes: 99 })` returns `{ enabled: true, maxVersionsPerFile: 5, autosaveIntervalMinutes: 60 }`.
- Assert key new translations exist and are non-empty in both `tr` and `en`: `titlebar.history`, `command.versionHistory`, `command.recoverDeleted`, `settings.historyEnabled`, `history.title`, `history.tabCurrentFile`, `history.tabDeletedFiles`, `history.restoreBtn`, `history.recoverOriginal`.

- [ ] **Step 2: Run test to verify it fails**

Run: `npm test`
Expected: FAIL (module `src/utils/historyUtils.ts` not found).

- [ ] **Step 3: Implement `historyUtils.ts`, update `electron.d.ts`, `tauriBridge.ts`, and `translations.ts`**

- Implement `src/utils/historyUtils.ts`.
- Add all DTO types and `ElectronAPI` methods in `src/types/electron.d.ts`.
- Update `writeFile` in `src/api/tauriBridge.ts` to pass `saveSource: saveSource || null` and add all 12 `history*` IPC bridge methods invoking the exact Rust command names from Task 2.
- Add all TR & EN translation keys in `src/i18n/translations.ts` with 1:1 parity.

- [ ] **Step 4: Run typecheck and JS tests**

Run: `npm run typecheck && npm test`
Expected: PASS (0 TS errors, all Node tests + i18n parity check pass).

- [ ] **Step 5: Commit**

```bash
git add src/utils/historyUtils.ts src/types/electron.d.ts src/api/tauriBridge.ts src/i18n/translations.ts test/test-utils.js
git commit -m "feat(history): add frontend bridge, types, utilities, and TR/EN i18n"
```

---

### Task 5: History & Deleted File Recovery Modal (`HistoryModal.tsx`) and Styles

**Files:**
- Create: `src/components/HistoryModal.tsx`
- Modify: `src/index.css` (add `--diff-*` theme tokens for dark & light themes and `.history-*` modal styles)

**Interfaces:**
- Consumes: `window.electronAPI` history methods, `HistoryTrigger`, `SnapshotSummary`, `DeletedFileSummary`, `DiffResult`, `RecoverFileResult`, `formatRelativeTimestamp`, `formatByteSize`, `buildCopyTabTitle`, `useTranslation`.
- Produces:
  ```ts
  export interface HistoryModalProps {
    isOpen: boolean;
    initialTab?: 'current' | 'deleted';
    activeFilePath?: string;
    activeFileTitle?: string;
    activeContent: string;
    onClose: () => void;
    onRestored: (filePath: string, restoredContent: string) => void;
    onOpenCopy: (title: string, content: string) => void;
    onRecovered: (result: RecoverFileResult) => void;
  }
  export const HistoryModal: React.FC<HistoryModalProps>;
  ```

- [ ] **Step 1: Add dark & light diff variables and modal layout CSS in `src/index.css`**

Define in `:root` (dark) and `[data-theme='light']`:
- `--diff-add-bg`, `--diff-add-text`, `--diff-del-bg`, `--diff-del-text`, `--diff-gutter-bg`, `--diff-gutter-text`.
Add clean scoped styles `.history-modal-box` (`width: 920px; max-width: 94vw; height: 76vh;`), `.history-body` (2-column split: left `270px` list, right flex viewer), `.history-item`, `.history-trigger-badge`, `.history-diff-viewer` (monospace `--font-editor`, line numbers, `+`/`-` markers, horizontal scroll), `.history-content-viewer`.

- [ ] **Step 2: Implement `src/components/HistoryModal.tsx`**

Implement per spec §4.4, §4.5, §4.7:
- Reset `activeTab` to `initialTab` when opened; load `historyListSnapshots(activeFilePath)` (if `activeFilePath` exists) and `historyListDeletedFiles()` in parallel; auto-select newest item.
- **Tab 1 (`Bu Dosya`):**
  - If `!activeFilePath`: show localized empty state explaining unsaved/untitled tabs have no file history yet.
  - Left list: snapshots showing `formatRelativeTimestamp`, trigger badge (`Manuel` / `Otomatik` / `Başlangıç` / `Geri Alma Öncesi`), line count & `formatByteSize(sizeBytes)`.
  - Right pane: view mode toggle (`Fark (Diff)` | `İçerik`), direction label (`Seçili sürüm → Güncel belge`), `+addedCount / -removedCount` summary badge, truncation warning banner if `diff.truncated`, scrollable diff/content view.
  - Footer buttons: **"Yeni Sekmede Kopya Aç"** (calls `historyGetSnapshotContent` -> `onOpenCopy(buildCopyTabTitle(...), content)` -> `onClose()`), **"Bu Sürüme Geri Dön"** (calls `historyRestoreSnapshot(activeFilePath, selected.id, activeContent)` -> `onRestored(activeFilePath, restored)` -> `onClose()`).
  - Also include subtle "Bu dosyanın geçmişini temizle" icon/action with confirmation if `snapshots.length > 0`.
- **Tab 2 (`Silinen / Kayıp Dosyalar`):**
  - Left list: deleted files with `fileName`, muted `filePath`, `formatRelativeTimestamp(updatedAt)`, and snapshot count.
  - Selecting a deleted file loads `historyListSnapshotsByFileId(fileId)` and lets the user pick which snapshot to inspect (dropdown or inline pills) and preview its full text content via `historyGetSnapshotContent`.
  - Footer buttons: **"Geçmişten Kaldır"** (confirmed `historyDeleteFileHistory`), **"Klasör Seçerek Kurtar..."** (calls `openDirectoryDialog` -> `historyRecoverFile(fileId, snapshotId, dir)` -> `onRecovered`), **"Orijinal Konumuna Kurtar"** (calls `historyRecoverFile(fileId, snapshotId, null)` -> `onRecovered`).
- Close on `Escape` key and overlay click; guard async state updates with cancellation flags to prevent stale responses when rapidly switching snapshots.

- [ ] **Step 3: Verify TypeScript compilation and production Vite build**

Run: `npm run typecheck && npm run build`
Expected: PASS (0 TS errors, Vite build succeeds).

- [ ] **Step 4: Commit**

```bash
git add src/components/HistoryModal.tsx src/index.css
git commit -m "feat(history): add HistoryModal with diff viewer and deleted file recovery"
```

---

### Task 6: Settings UI Controls (`SettingsModal.tsx`)

**Files:**
- Modify: `src/components/SettingsModal.tsx:20-60,193-315`

**Interfaces:**
- Consumes: `HistorySettings`, `HistoryStats`, `formatByteSize`, `clampHistorySettings`, `useTranslation`.
- Produces: Extended `SettingsModalProps`:
  - `historySettings?: HistorySettings`
  - `historyStats?: HistoryStats | null`
  - `onUpdateHistorySettings?: (next: HistorySettings) => Promise<void>`
  - `onClearAllHistory?: () => Promise<void>`

- [ ] **Step 1: Add Version History configuration card to `SettingsModal.tsx` (`activeTab === 'files'`)**

Below the existing `filterMdOnly` row inside `activeTab === 'files'`, add a styled card matching `defaultSavePath`'s existing container (`var(--bg-input)` / `var(--border-default)`):
- Section header with `History` icon (`lucide-react`) + `t('settings.historyTitle')`.
- **Enable toggle** (`historySettings.enabled`).
- **Max versions per file** (`5–200`, default `50`): numeric `<input type="number" min={5} max={200} />` + quick preset buttons (`20`, `50`, `100`, `200`), clamped via `clampHistorySettings` and saved via `onUpdateHistorySettings`.
- **Auto-save snapshot interval**: `<select>` with options `1, 3, 5, 10, 15, 30` minutes (disabled when `!historySettings.enabled`), saved via `onUpdateHistorySettings`.
- **Storage stats & Clear All footer**: displays `t('settings.historyStorageStats', { files, snapshots, size: formatByteSize(dbSizeBytes) })` and danger secondary button `t('settings.historyClearAll')` which asks `window.confirm(t('settings.historyClearAllConfirm'))` before invoking `onClearAllHistory()`.

- [ ] **Step 2: Verify TypeScript compilation and Vite build**

Run: `npm run typecheck && npm run build`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src/components/SettingsModal.tsx
git commit -m "feat(history): add version history settings and storage stats in SettingsModal"
```

---

### Task 7: App Integration (`TitleBar.tsx` & `App.tsx`) and Full Verification

**Files:**
- Modify: `src/components/TitleBar.tsx:1-60,150-185`
- Modify: `src/App.tsx` (state, `writeFile` `saveSource` tags, `HistoryModal`, `SettingsModal`, `TitleBar`, `CommandPalette`, `onMenuCommand`, web shortcut)

**Interfaces:**
- Consumes: `HistoryModal` (Task 5), `SettingsModal` history props (Task 6), `window.electronAPI` history methods (Task 4).
- Produces: Complete user-facing integration across all entry points and save paths.

- [ ] **Step 1: Add `History` icon button to `src/components/TitleBar.tsx`**

Add `onOpenHistory?: () => void;` to `TitleBarProps` and render a `.btn-icon` button with `<History size={16} />` (`title={t('titlebar.history')}`) right next to Export/OCR/Settings.

- [ ] **Step 2: Wire `src/App.tsx` across all save paths, entry points, and modals**

1. **State:**
   - `historyModalOpen: boolean`, `historyModalInitialTab: 'current' | 'deleted'`
   - `historySettings: HistorySettings` (defaults `{ enabled: true, maxVersionsPerFile: 50, autosaveIntervalMinutes: 3 }`), `historyStats: HistoryStats | null`
2. **Load settings & stats:**
   - On mount (when `window.electronAPI?.historyGetSettings`), load `historySettings`.
   - When `settingsModalOpen` becomes `true`, refresh `historySettings` and `historyStats`.
   - Implement `handleUpdateHistorySettings(next)` (calls `historyUpdateSettings`, updates state, refreshes `historyStats`) and `handleClearAllHistory()` (calls `historyClearAll`, refreshes `historyStats`).
3. **Explicit `saveSource` on every `writeFile` call site in `App.tsx`:**
   - Auto-save `useEffect` (~line 345): `window.electronAPI.writeFile(finalPath, contentToWrite, 'autosave')`
   - `handleNewFile` (~line 440): `window.electronAPI.writeFile(finalFilePath, initialContent, 'initial')`
   - `handleOCRComplete` (~line 535): `window.electronAPI.writeFile(finalFilePath, markdown, 'initial')`
   - `handleSaveFile` (~line 661): `window.electronAPI.writeFile(targetPath, contentToWrite, 'manual')`
   - `handleSaveAllAndClose` (~line 752): `window.electronAPI.writeFile(targetPath, tab.content, 'manual')`
4. **History modal callbacks:**
   - `handleHistoryRestored(filePath, restoredContent)`: update matching tab(s) (`t.filePath === filePath`) with `content: restoredContent, savedContent: restoredContent, isModified: false` and focus editor.
   - `handleHistoryOpenCopy(title, content)`: create new tab (`id: createTabId()`, `title`, `filePath: undefined`, `content`, `savedContent: ''`, `isModified: true`) and activate it.
   - `handleHistoryRecovered(result)`: call `openFileInTab(result.filePath, result.fileName, result.content)`; if `rootFolderPath && result.filePath.startsWith(rootFolderPath)`, call `handleRefreshTree()`.
5. **Entry points:**
   - `TitleBar`: pass `onOpenHistory={() => { setHistoryModalInitialTab('current'); setHistoryModalOpen(true); }}`.
   - `onMenuCommand`: handle `case 'version-history': setHistoryModalInitialTab('current'); setHistoryModalOpen(true); break;`.
   - Keyboard shortcut (`CmdOrCtrl+Shift+H` in web fallback).
   - `CommandPalette`: add `cmd-version-history` (`Cmd+Shift+H`, opens `'current'`) and `cmd-recover-deleted` (opens `'deleted'`).
   - Mount `<HistoryModal ... />` and pass history props to `<SettingsModal ... />`.

- [ ] **Step 3: Run full verification suite (frontend + backend)**

Run:
1. `npm run typecheck` — Expected: exit 0, 0 errors.
2. `npm test` — Expected: all tests pass including `historyUtils` and 1:1 TR/EN i18n parity.
3. `npm run build` — Expected: clean Vite production bundle.
4. `cargo test --manifest-path src-tauri/Cargo.toml` — Expected: all Rust unit & integration tests pass with 0 failures.

- [ ] **Step 4: Commit**

```bash
git add src/components/TitleBar.tsx src/App.tsx
git commit -m "feat(history): integrate version history modal, settings, and save hooks in App"
```
