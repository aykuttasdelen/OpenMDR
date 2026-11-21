# Google Drive Bulut Yedekleme (Aşama 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build one-way automatic Google Drive cloud backup (`Local -> Drive` into visible `OpenMD/` folder hierarchy), macOS Keychain PKCE OAuth, `Cmd+Q`/window-close flush, and on-demand restore (`DriveRestoreModal`) for the OpenMD Tauri desktop app.

**Architecture:** All OAuth 2.0 PKCE loopback handling (`drive_auth.rs`), token persistence (`TokenStore` trait backed by macOS Keychain `keyring` and in-memory test mock), Google Drive REST v3 calls (`drive_client.rs`), and the dirty-queue sync engine (`drive_sync.rs`) live in Rust behind Tauri IPC commands (`drive_ops.rs`). The existing `history.db` SQLite database is migrated safely from `user_version = 1` to `user_version = 2` to track Drive folders, files, SHA-256 sync state, and Drive settings without modifying any Phase 1 history tables or relaxing `tauri.conf.json` CSP. On the frontend, `tauriBridge.ts`, `SettingsModal` (`cloud` tab), `StatusBar`, `DriveRestoreModal`, `DriveQuitSyncModal`, and `App.tsx` surface desktop-only cloud backup UX while remaining hidden in web mode (`!isTauri()`).

**Tech Stack:** Rust (edition 2024, MSRV 1.90), Tauri v2 (`2.12.5`), `tokio`, `rusqlite` (`bundled`), `sha2`, `reqwest` (`0.12`, `rustls-tls`), `keyring` (`3`, `apple-native`), `rand` (`0.9`), React 19, TypeScript 5.8, Vite 6, Lucide icons, Node native test runner (`node --experimental-strip-types`).

**Spec:** `docs/superpowers/specs/2026-11-21-google-drive-backup-design.md`

## Global Constraints

- **Desktop-only feature:** All Google Drive UI controls, menus, command palette actions, and status badges MUST be hidden and skipped when `!isTauri()`.
- **No CSP relaxation:** `src-tauri/tauri.conf.json` `app.security.csp` (`connect-src 'self' ipc: http://ipc.localhost ws://localhost:5173`) MUST NOT be modified.
- **Credential & secret hygiene:** Never hardcode OAuth `client_id`, `client_secret`, or tokens in tracked source files or docs, never store tokens/secrets in `localStorage` or WebView JS memory, and never return secrets over Tauri IPC. Local builds read `OPENMD_GOOGLE_CLIENT_ID` and `OPENMD_GOOGLE_CLIENT_SECRET` via `build.rs` from process env or gitignored `.env.local`.
- **Non-blocking local writes:** Local file writes (`write_file`, `create_file`, `rename_file`, `save_file_dialog`, `restore_snapshot`, `recover_file`) MUST never fail or block if Drive auth, network, or SQLite queueing fails.
- **No auto-delete in Drive:** Local `delete_file` MUST never delete remote Drive backups.
- **File eligibility & cap:** Eligible backup files are `.md`, `.mdx`, `.markdown` (case-insensitive), valid UTF-8, and `<= 10 * 1024 * 1024` bytes (`10 MiB`) for both upload and download.
- **Timestamps & JSON naming:** All SQLite timestamps and IPC numeric timestamps are Unix milliseconds (`i64`); all Rust IPC DTOs use `#[serde(rename_all = "camelCase")]`.
- **Bilingual parity:** Every key added to `src/i18n/translations.ts` MUST exist non-empty in both `tr` and `en` (enforced by `npm test`).
- **Rust quality gate:** `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` MUST exit 0 with 0 warnings.

## Review Focus

1. **Non-destructive v1 -> v2 SQLite migration (`PRAGMA user_version`):** Upgrading an existing Phase 1 `history.db` (`user_version = 1`) containing real snapshots and settings must upgrade cleanly to `user_version = 2` without dropping or altering existing `tracked_files`, `snapshots`, or `settings` rows. *(Pinned in Task 1)*
2. **Drive `q` query escaping & UTF-8 names:** Folders and filenames containing single quotes, backslashes, or Turkish characters (`O'Reilly'nin Notları/Türkçe.md`) must be escaped cleanly in Drive queries and preserved in multipart uploads. *(Pinned in Task 3)*
3. **Content-unchanged rename/move sync:** Renaming a backed-up file or parent directory on disk when file content bytes did not change (`SHA-256 == last_synced_hash`) must still update `last_synced_remote_path` in SQLite and move/rename the remote Drive file via `addParents`/`removeParents`. *(Pinned in Task 4)*
4. **Concurrent refresh serialization & `invalid_grant` lockout:** Multiple simultaneous callers needing an expired access token must perform only one refresh under `refresh_lock`, and an `invalid_grant` response must transition state to `auth_required` and halt background retries until the user reconnects. *(Pinned in Task 2 & Task 4)*
5. **Restore path-traversal & collision safety:** Remote Drive paths containing `.` / `..` / `/` / `\` / NUL segments must be rejected before disk write, and colliding filenames in `targetDir` must generate collision-free names via `resolve_available_path` rather than overwriting existing local files. *(Pinned in Task 5)*

---

## File Structure

**Created:**
- `src-tauri/src/drive_auth.rs` — `TokenStore` trait (`KeychainTokenStore` & `MemoryTokenStore`), OAuth 2.0 PKCE + loopback listener (`127.0.0.1:0`), credential resolution (BYO > build-time env), serialized token refresh, revoke/disconnect, and mock-server unit tests.
- `src-tauri/src/drive_client.rs` — Configurable-endpoint Google Drive REST v3 client (`reqwest`), `q` literal escaping, RFC 3339 parser, folder ensure + 404 self-healing cache, multipart upload, media PATCH, `addParents`/`removeParents` move/rename, paginated remote file listing, 10 MiB download guard, and mock HTTP tests.
- `src-tauri/src/drive_sync.rs` — `DriveManager` (`DriveState`), `compute_remote_path`, 8-state status model, background `tokio` worker (30s idle + 5m periodic + backoff + 2s startup grace), workspace/open-tab scanner, quit flush + terminate fallback, `drive:status-changed` emitter, and sync engine tests.
- `src-tauri/src/commands/drive_ops.rs` — 13 Tauri IPC commands + traversal-safe/collision-safe `restore_remote_file_inner` & `restore_all_remote_files_inner` + restore tests.
- `src/utils/driveUtils.ts` — Pure TS helpers (`clampDriveSettings`, `filterRemoteFiles`, `mapDriveErrorCode`, `getDriveStatusTone`).
- `src/components/DriveRestoreModal.tsx` — Live Drive `OpenMD/` file browser, search, content preview, open-in-new-tab, single restore & batch download modal.
- `src/components/DriveQuitSyncModal.tsx` — Quit-flush progress/timeout/retry modal with "Beklemeden Kapat", "Tekrar Dene", and "İptal".

**Modified:**
- `src-tauri/Cargo.toml` — Add `reqwest` (`0.12`, `rustls-tls`), `keyring` (`3`, `apple-native`), `rand` (`0.9`).
- `src-tauri/build.rs` — Inject optional `OPENMD_GOOGLE_CLIENT_ID` / `OPENMD_GOOGLE_CLIENT_SECRET` at compile time from process env or `../.env.local`.
- `src-tauri/src/history_store.rs` — Version-checked `v1 -> v2` migration (`drive_folders`, `drive_files`, `drive_*` settings) and SQLite Drive metadata/queue CRUD methods + migration/queue unit tests.
- `src-tauri/src/commands/fs_ops.rs` — Best-effort `DriveManager` hooks in `write_file`, `create_file`, `rename_file`.
- `src-tauri/src/commands/dialogs.rs` — Best-effort `DriveManager` hook in `save_file_dialog`.
- `src-tauri/src/commands/history_ops.rs` — Best-effort `DriveManager` hooks in `restore_snapshot` & `recover_file`.
- `src-tauri/src/commands/mod.rs` — Export `pub mod drive_ops;`.
- `src-tauri/src/menu.rs` — Replace `PredefinedMenuItem::quit` with `"quit-app"` (`CmdOrCtrl+Q`) routed to main window close flow; add `drive-sync-now` and `drive-restore` File menu items.
- `src-tauri/src/lib.rs` — Wire `drive_auth`, `drive_client`, `drive_sync`; initialize `DriveState`; handle `RunEvent::ExitRequested` & `RunEvent::Exit` fallback; register 13 `drive_ops` commands.
- `src/types/electron.d.ts` — Add Drive DTO types and `ElectronAPI` methods.
- `src/api/tauriBridge.ts` — Implement all 13 `drive*` IPC wrappers and `onDriveStatusChanged`.
- `src/i18n/translations.ts` — Add matching `tr` and `en` keys for Cloud settings, StatusBar, Restore modal, Quit-flush modal, menus, and errors.
- `src/index.css` — Styles for StatusBar cloud pill, Settings cloud card/accordion, `DriveRestoreModal`, and `DriveQuitSyncModal`.
- `src/components/SettingsModal.tsx` — Add `initialTab` prop and 5th `cloud` tab (gated on Drive callbacks).
- `src/components/StatusBar.tsx` — Add clickable Drive status pill beside `AutoSave` (rendered only when `driveStatus` prop is provided).
- `src/App.tsx` — Manage `driveStatus`, event subscription, 2-stage quit interception, menu/Command Palette triggers, and restore/tab/tree refresh integration.
- `test/test-utils.js` — Add unit tests for `driveUtils.ts`.

---

### Task 1: SQLite v1 -> v2 Migration & Drive Metadata Store (`history_store.rs`)

**Files:**
- Modify: `src-tauri/src/history_store.rs`

**Interfaces:**
- Consumes: `rusqlite::{Connection, OptionalExtension, params}`, `HistoryStore::normalize_path`.
- Produces in `src-tauri/src/history_store.rs`:
  - Constants:
    - `DEFAULT_DRIVE_SYNC_INTERVAL_MINUTES: u32 = 5`, `MIN_DRIVE_SYNC_INTERVAL_MINUTES: u32 = 1`, `MAX_DRIVE_SYNC_INTERVAL_MINUTES: u32 = 60`
    - `DEFAULT_DRIVE_IDLE_DELAY_SECONDS: u32 = 30`, `MIN_DRIVE_IDLE_DELAY_SECONDS: u32 = 10`, `MAX_DRIVE_IDLE_DELAY_SECONDS: u32 = 300`
  - Structs (`#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`, `#[serde(rename_all = "camelCase")]`):
    - `DriveSettings { pub enabled: bool, pub sync_interval_minutes: u32, pub idle_delay_seconds: u32 }`
    - `DriveFileRecord { pub id: i64, pub local_path: String, pub remote_path: String, pub drive_file_id: Option<String>, pub drive_parent_id: Option<String>, pub last_synced_hash: Option<String>, pub last_synced_remote_path: Option<String>, pub last_synced_at: Option<i64>, pub dirty: bool, pub dirty_since: Option<i64>, pub retry_count: i64, pub next_retry_at: i64, pub last_error: Option<String>, pub updated_at: i64 }`
    - `DriveStoreSummary { pub pending_count: i64, pub synced_count: i64, pub last_sync_at: Option<i64> }`
  - `HistoryStore` methods:
    - `pub fn get_drive_settings(&self) -> Result<DriveSettings, String>`
    - `pub fn update_drive_settings(&self, settings: DriveSettings) -> Result<DriveSettings, String>`
    - `pub fn get_drive_setting_str(&self, key: &str) -> Result<String, String>`
    - `pub fn set_drive_setting_str(&self, key: &str, value: &str) -> Result<(), String>`
    - `pub fn get_cached_folder_id(&self, remote_folder_path: &str) -> Result<Option<String>, String>`
    - `pub fn upsert_cached_folder(&self, remote_folder_path: &str, drive_folder_id: &str, parent_drive_id: Option<&str>, now_ms: i64) -> Result<(), String>`
    - `pub fn invalidate_folder_prefix(&self, remote_folder_path: &str) -> Result<(), String>`
    - `pub fn upsert_drive_file_dirty(&self, local_path: &str, remote_path: &str, now_ms: i64) -> Result<(), String>`
    - `pub fn upsert_drive_file_dirty_if_needed(&self, local_path: &str, remote_path: &str, content_hash: &str, now_ms: i64) -> Result<bool, String>` *(inserts/marks `dirty = 1` only when missing OR `last_synced_hash != Some(content_hash)` OR `last_synced_remote_path != Some(remote_path)`)*
    - `pub fn get_drive_file(&self, local_path: &str) -> Result<Option<DriveFileRecord>, String>`
    - `pub fn list_due_dirty_files(&self, max_next_retry_at_ms: i64) -> Result<Vec<DriveFileRecord>, String>`
    - `pub fn mark_drive_file_synced(&self, local_path: &str, remote_path: &str, drive_file_id: &str, drive_parent_id: &str, content_hash: &str, now_ms: i64) -> Result<(), String>`
    - `pub fn mark_drive_file_clean(&self, local_path: &str, now_ms: i64) -> Result<(), String>`
    - `pub fn mark_drive_file_retry(&self, local_path: &str, retry_count: i64, next_retry_at_ms: i64, error: &str, now_ms: i64) -> Result<(), String>`
    - `pub fn clear_drive_file_remote_ids(&self, local_path: &str, now_ms: i64) -> Result<(), String>` *(clears `drive_file_id` and `drive_parent_id` for 404 self-healing)*
    - `pub fn handle_drive_rename(&self, old_path: &str, new_path: &str, remote_mapper: impl Fn(&str) -> Result<String, String>, now_ms: i64) -> Result<usize, String>`
    - `pub fn clear_all_drive_cache(&self) -> Result<(), String>` *(clears `drive_files`, `drive_folders`, resets `drive_account_email=''`, `drive_root_folder_id=''`, `drive_last_sync_at='0'`, `drive_enabled='false'`)*
    - `pub fn get_drive_summary(&self) -> Result<DriveStoreSummary, String>`

- [ ] **Step 1: Write failing v1 -> v2 migration and Drive store tests in `src-tauri/src/history_store.rs`**

Add stub types/methods returning `Err("todo".to_string())` and append these 5 unit tests inside `mod tests` in `src-tauri/src/history_store.rs`:
1. `test_migrate_v1_to_v2_preserves_existing_history`: manually creates an old `user_version = 1` SQLite database at `dir.path().join("history.db")` containing 1 custom `settings` row (`max_versions_per_file = 42`), 1 `tracked_files` row, and 1 compressed `snapshots` row; calls `HistoryStore::open(db_path)`; asserts `PRAGMA user_version == 2`, Phase 1 `get_settings().max_versions_per_file == 42`, `list_snapshots` still returns 1 snapshot, and default `get_drive_settings()` returns `{ enabled: false, sync_interval_minutes: 5, idle_delay_seconds: 30 }`.
2. `test_drive_settings_roundtrip_and_validation`: validates valid updates (`enabled: true, sync_interval_minutes: 15, idle_delay_seconds: 60`) persist across reopen and invalid bounds (`sync_interval_minutes: 0` or `61`, `idle_delay_seconds: 9` or `301`) return `Err`.
3. `test_drive_file_dirty_queue_and_backoff_filtering`: calls `upsert_drive_file_dirty` for two files, marks file A retry with `next_retry_at_ms = 50_000`, asserts `list_due_dirty_files(10_000)` returns only file B and `list_due_dirty_files(i64::MAX)` returns both; marks file B synced (`mark_drive_file_synced`), verifies `get_drive_summary()` reports `synced_count == 1, pending_count == 1, last_sync_at == Some(...)`, and verifies `upsert_drive_file_dirty_if_needed` returns `false` for matching hash+remote_path and `true` when hash changes.
4. `test_drive_folder_cache_and_prefix_invalidation`: inserts cached folders `""` (`root-id`), `"Documents"` (`docs-id`), `"Documents/Notes"` (`notes-id`), and `"Documents2"` (`docs2-id`); calls `invalidate_folder_prefix("Documents")`; asserts `"Documents"` and `"Documents/Notes"` are deleted while `""` and `"Documents2"` remain intact. Also tests invalidating `""` clears all cached folders.
5. `test_handle_drive_rename_file_and_directory_prefix`: inserts `drive_files` rows for `/u/docs/a.md`, `/u/docs/sub/b.md`, `/u/docs2/c.md`, plus an already-existing target collision row `/u/notes/a.md` (`file-old`); calls `handle_drive_rename("/u/docs", "/u/notes", ...)`; asserts `/u/docs/a.md` became `/u/notes/a.md` (`dirty == true`), old collision row was cleanly replaced without `UNIQUE` constraint error, `/u/docs/sub/b.md` became `/u/notes/sub/b.md`, and sibling `/u/docs2/c.md` stayed untouched with `dirty` unchanged.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml test_migrate_v1_to_v2`
Expected: FAIL with `"todo"` / assertion errors.

- [ ] **Step 3: Implement `v1 -> v2` migration and Drive SQLite store methods in `src-tauri/src/history_store.rs`**

Update `HistoryStore::open`:
- Configure `PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;`
- Read `let user_version: i64 = conn.query_row("PRAGMA user_version;", [], |row| row.get(0))?;`
- If `user_version < 1`, run v1 schema + v1 default settings inside `conn.transaction()` and set `PRAGMA user_version = 1;`
- If `user_version < 2`, run v2 schema (`drive_folders`, `drive_files`, `idx_drive_files_dirty`) + `INSERT OR IGNORE` default `drive_*` settings inside `conn.transaction()` and set `PRAGMA user_version = 2;`
Implement all Drive settings, folder cache, dirty queue, rename prefix helper (`HistoryStore::normalize_path` + separator boundary check), summary, and cache-clear methods matching the spec schema.

- [ ] **Step 4: Run all tests in `history_store` to verify they pass**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml history_store`
Expected: PASS (all Phase 1 tests + 5 new Phase 2 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/history_store.rs
git commit -m "feat(drive): migrate history.db to v2 with Drive metadata and queue store"
```

---

### Task 2: Dependencies, Build-Time OAuth Env Loader, Keychain `TokenStore`, & PKCE Loopback Auth (`drive_auth.rs`)

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/build.rs`
- Create: `src-tauri/src/drive_auth.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod drive_auth;`)

**Interfaces:**
- Consumes: `HistoryStore` (`get_drive_setting_str`, `set_drive_setting_str`, `clear_all_drive_cache`), `tokio::net::TcpListener`, `tokio::sync::{watch, Mutex}`, `reqwest::Client`, `keyring::Entry`, `sha2::Sha256`, `base64::engine::general_purpose::URL_SAFE_NO_PAD`, `rand`.
- Produces in `src-tauri/src/drive_auth.rs`:
  - `pub trait TokenStore: Send + Sync`:
    - `fn load_tokens(&self) -> Result<Option<StoredOAuthTokens>, String>`
    - `fn save_tokens(&self, tokens: &StoredOAuthTokens) -> Result<(), String>`
    - `fn delete_tokens(&self) -> Result<(), String>`
    - `fn load_custom_secret(&self) -> Result<Option<String>, String>`
    - `fn save_custom_secret(&self, secret: &str) -> Result<(), String>`
    - `fn delete_custom_secret(&self) -> Result<(), String>`
  - `pub struct KeychainTokenStore;` (uses `keyring::Entry::new("com.openmd.editor", ...)`, accounts `"google-drive-oauth"` & `"google-drive-custom-client-secret"`)
  - `pub struct MemoryTokenStore` (thread-safe test double)
  - `pub struct OAuthEndpoints { pub auth_url: String, pub token_url: String, pub revoke_url: String, pub userinfo_url: String }` (`Default` uses official Google URLs)
  - `pub struct OAuthClientCredentials { pub client_id: String, pub client_secret: Option<String>, pub is_custom: bool }`
  - `pub struct DriveAuthConfig { pub has_built_in_client: bool, pub using_custom_client: bool, pub custom_client_id: String, pub has_custom_client_secret: bool }` (`#[serde(rename_all = "camelCase")]`)
  - `pub struct StoredOAuthTokens { pub refresh_token: String, pub access_token: String, pub expires_at_ms: i64, pub email: String }` (`#[serde(rename_all = "camelCase")]`)
  - `pub enum AuthError { NotConfigured, Cancelled, Timeout, Denied(String), StateMismatch, InvalidGrant(String), Network(String), Other(String) }`
  - `pub struct DriveAuthManager`:
    - `pub fn new(store: Arc<HistoryStore>, token_store: Arc<dyn TokenStore>, http: reqwest::Client, endpoints: OAuthEndpoints) -> Self`
    - `pub fn with_built_in_credentials(self, client_id: Option<String>, client_secret: Option<String>) -> Self`
    - `pub fn resolve_credentials(&self) -> Result<OAuthClientCredentials, AuthError>`
    - `pub fn get_auth_config(&self) -> Result<DriveAuthConfig, String>`
    - `pub fn save_custom_oauth(&self, client_id: &str, client_secret: Option<&str>) -> Result<(DriveAuthConfig, bool), String>` *(returns `(config, credentials_changed)`)*
    - `pub fn clear_custom_oauth(&self) -> Result<(DriveAuthConfig, bool), String>`
    - `pub fn has_stored_tokens(&self) -> bool`
    - `pub fn is_auth_in_progress(&self) -> bool`
    - `pub async fn bind_loopback_session<F>(&self, open_url: F) -> Result<(StoredOAuthTokens, String), AuthError>` where `F: FnOnce(&str) -> Result<(), String>` *(accepts closure so Tauri command opens browser via `OpenerExt` and tests connect directly with `reqwest`!)*
    - `pub fn cancel_auth(&self) -> bool`
    - `pub async fn get_valid_access_token(&self) -> Result<String, AuthError>` *(serialized by `refresh_lock`, refreshes if `now_ms + 60_000 >= expires_at_ms`, detects `"invalid_grant"`)*
    - `pub async fn revoke_and_clear_tokens(&self) -> Result<(), String>`
    - `pub fn generate_pkce_pair() -> (String, String)` and `pub fn generate_state() -> String`

- [ ] **Step 1: Update `src-tauri/Cargo.toml` and `src-tauri/build.rs`**

Add to `[dependencies]` in `src-tauri/Cargo.toml`:
```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
keyring = { version = "3", features = ["apple-native"] }
rand = "0.9"
```
Update `src-tauri/build.rs` to check `OPENMD_GOOGLE_CLIENT_ID` and `OPENMD_GOOGLE_CLIENT_SECRET`:
- Emit `cargo:rerun-if-env-changed=OPENMD_GOOGLE_CLIENT_ID`, `cargo:rerun-if-env-changed=OPENMD_GOOGLE_CLIENT_SECRET`, and `cargo:rerun-if-changed=../.env.local`.
- If either env var is absent in process env, read `../.env.local` line by line (`KEY=VALUE`, strip optional quotes) and if non-empty emit `cargo:rustc-env=KEY=VALUE`. Never commit `.env.local`.
Add `pub mod drive_auth;` to `src-tauri/src/lib.rs`.

- [ ] **Step 2: Write failing OAuth/PKCE unit tests in `src-tauri/src/drive_auth.rs`**

Create `src-tauri/src/drive_auth.rs` with type stubs and these 7 unit tests:
1. `test_pkce_s256_challenge_and_state`: generates 2 PKCE pairs and states; asserts verifier length >= 43, URL-safe base64 without `=`, `challenge == URL_SAFE_NO_PAD(Sha256(verifier))`, and distinct random values.
2. `test_oauth_credential_resolution_byo_overrides_builtin`: configures built-in ID+secret, verifies `resolve_credentials()` returns built-in; saves BYO client via `save_custom_oauth("byo-id", Some("byo-sec"))`, asserts `resolve_credentials()` returns BYO (`is_custom == true`), `credentials_changed == true`, and `get_auth_config()` hides secret while setting `has_custom_client_secret == true`. Clearing returns built-in. When neither exists, returns `Err(AuthError::NotConfigured)`.
3. `test_loopback_oauth_happy_path_with_mock_server`: spawns local mock `TcpListener` HTTP server serving `POST /token` + `GET /userinfo`, launches `bind_loopback_session` whose `open_url` inspects query params (`code_challenge_method=S256`, `access_type=offline`, `prompt=consent`, `scope` containing `drive.file`, `openid`, `email`, extracted `redirect_uri` & `state`) and hits the loopback callback URL with `?code=auth-code-123&state=<state>`. Asserts browser HTTP response is 200 HTML, tokens are saved in `MemoryTokenStore`, `token_request` included `code_verifier` & `client_secret`, and email `"alice@example.com"` is returned.
4. `test_loopback_oauth_rejects_state_mismatch_and_access_denied`: calls loopback callback once with wrong `state` -> asserts `Err(AuthError::StateMismatch)` and 400 HTML; resets and calls callback with `?error=access_denied` -> asserts `Err(AuthError::Denied(_))`.
5. `test_loopback_oauth_cancellation`: starts `bind_loopback_session` and triggers `cancel_auth()`; asserts future resolves promptly with `Err(AuthError::Cancelled)` and `is_auth_in_progress() == false`.
6. `test_token_refresh_serialization_and_invalid_grant`: seeds expired token (`expires_at_ms = 0`) in `MemoryTokenStore`; spawns mock token endpoint counting requests via `AtomicUsize`; runs 4 concurrent `get_valid_access_token()` calls via `tokio::join!`; asserts token endpoint was hit **once**, refreshed token persisted while keeping existing `refresh_token`. Then repeats with `invalid_grant` 400 JSON and asserts `Err(AuthError::InvalidGrant(_))`.
7. `test_revoke_and_clear_tokens`: seeds token, calls `revoke_and_clear_tokens()`, verifies mock `POST /revoke` received token and `MemoryTokenStore::load_tokens()` is `None`.

- [ ] **Step 3: Run tests to verify failure**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml drive_auth`
Expected: FAIL with unimplemented stubs.

- [ ] **Step 4: Implement `src-tauri/src/drive_auth.rs`**

Implement PKCE generation, `TcpListener` (`127.0.0.1:0`) HTTP request/response handling with percent-decoding & 180s timeout, `tokio::sync::watch` cancellation (cancelling any prior active auth session before starting a new one), `refresh_lock` serialization, proactive refresh (`now_ms + 60_000 >= expires_at_ms`), `invalid_grant` classification, Keychain persistence (`com.openmd.editor`), and best-effort revoke.

- [ ] **Step 5: Run `drive_auth` tests and verify pass**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml drive_auth`
Expected: PASS (7/7 tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/build.rs src-tauri/src/drive_auth.rs src-tauri/src/lib.rs
git commit -m "feat(drive): add PKCE loopback OAuth, build-time env loader, and Keychain TokenStore"
```

---

### Task 3: Google Drive REST v3 HTTP Client (`drive_client.rs`)

**Files:**
- Create: `src-tauri/src/drive_client.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod drive_client;`)

**Interfaces:**
- Consumes: `HistoryStore` (`get_cached_folder_id`, `upsert_cached_folder`, `invalidate_folder_prefix`, `set_drive_setting_str`), `reqwest::Client`.
- Produces in `src-tauri/src/drive_client.rs`:
  - Constants: `MAX_BACKUP_BYTES: usize = 10 * 1024 * 1024`, `DRIVE_ROOT_FOLDER_NAME: &str = "OpenMD"`, `DRIVE_FOLDER_MIME: &str = "application/vnd.google-apps.folder"`, `MARKDOWN_MIME: &str = "text/markdown; charset=utf-8"`, `MAX_LIST_ITEMS: usize = 5000`.
  - Structs (`#[serde(rename_all = "camelCase")]`):
    - `DriveApiEndpoints { pub api_base_url: String, pub upload_base_url: String }` (`Default`: `https://www.googleapis.com/drive/v3` and `https://upload.googleapis.com/upload/drive/v3`)
    - `DriveRemoteFile { pub file_id: String, pub name: String, pub remote_path: String, pub size_bytes: u64, pub modified_time_ms: Option<i64> }`
    - `UploadedFileMeta { pub id: String, pub parent_id: String, pub modified_time_ms: Option<i64> }`
    - `DriveApiError { Unauthorized(String), NotFound(String), RateLimited(String), Network(String), TooLarge(usize), Api { status: u16, message: String } }`
  - `DriveClient` methods:
    - `pub fn new(http: reqwest::Client) -> Self` and `pub fn with_endpoints(http: reqwest::Client, endpoints: DriveApiEndpoints) -> Self`
    - `pub fn escape_drive_query_literal(value: &str) -> String` *(escapes `\` -> `\\` and `'` -> `\'`)*
    - `pub fn parse_rfc3339_to_unix_ms(value: &str) -> Option<i64>` *(pure std UTC Gregorian civil->epoch converter without extra date crates)*
    - `pub async fn ensure_root_folder(&self, access_token: &str, store: &HistoryStore, now_ms: i64) -> Result<String, DriveApiError>`
    - `pub async fn ensure_folder_chain(&self, access_token: &str, store: &HistoryStore, folder_segments: &[&str], now_ms: i64) -> Result<String, DriveApiError>`
    - `pub async fn upload_or_update_file(&self, access_token: &str, existing_file_id: Option<&str>, existing_parent_id: Option<&str>, target_parent_id: &str, file_name: &str, content: &str, rename_or_move: bool) -> Result<UploadedFileMeta, DriveApiError>` *(checks existing name in parent when `existing_file_id` is `None`; uses multipart POST on create, `PATCH /drive/v3/files/{id}?addParents=...&removeParents=...` on move/rename, and `PATCH uploadType=media` on content update)*
    - `pub async fn list_files_under_root(&self, access_token: &str, root_folder_id_hint: Option<&str>) -> Result<Vec<DriveRemoteFile>, DriveApiError>` *(paginated BFS over folder IDs up to 5000 items, filtering `.md`/`.mdx`/`.markdown`, sorted by `modifiedTimeMs DESC`)*
    - `pub async fn download_file_content(&self, access_token: &str, file_id: &str) -> Result<String, DriveApiError>` *(enforces `<= MAX_BACKUP_BYTES` before decoding UTF-8)*

- [ ] **Step 1: Add `pub mod drive_client;` to `src-tauri/src/lib.rs` and write failing tests in `src-tauri/src/drive_client.rs`**

Create `src-tauri/src/drive_client.rs` with stub signatures and these 7 unit/integration tests backed by a local mock `tokio::net::TcpListener` server:
1. `test_escape_drive_query_literal_quotes_and_backslashes`: verifies `O'Reilly\Docs` becomes `O\'Reilly\\Docs`.
2. `test_parse_rfc3339_to_unix_ms`: verifies `"2026-11-21T15:30:45.123Z"` and `"1970-01-01T00:00:00Z"` (`0`) and leap year `"2024-02-29T12:00:00Z"` parse to exact Unix milliseconds.
3. `test_ensure_folder_chain_caches_and_heals_404`: first call resolves `OpenMD` + `Documents` + `Notes`, populates `drive_folders`; second call uses DB cache without hitting mock server; deleting `Notes` or triggering 404 self-heals via `invalidate_folder_prefix`.
4. `test_upload_multipart_and_media_patch_with_turkish_and_quotes`: uploads `"O'Reilly'nin Notları.md"` (`"İçerik\n"`), checks mock server received multipart JSON with escaped metadata + UTF-8 body; second call with `existing_file_id` sends `PATCH uploadType=media`. Also verifies finding an existing file with same name in parent updates via PATCH instead of duplicate create.
5. `test_move_or_rename_uses_add_remove_parents_query_params`: calls `upload_or_update_file` with `existing_file_id: Some("f1")`, `existing_parent_id: Some("p1")`, `target_parent_id: "p2"`, `file_name: "Yeni Ad.md"`, `rename_or_move: true`; asserts mock server received `PATCH /files/f1` with `addParents=p2`, `removeParents=p1`, and JSON body `{"name":"Yeni Ad.md"}` (never `"parents"` in PATCH body!), followed by media PATCH.
6. `test_list_files_under_root_paginated_bfs`: mock server serves root `OpenMD` + paginated child folder (`pageToken=p2`) + 2 `.md`/`.mdx` files and 1 `.png` asset; asserts returned `Vec<DriveRemoteFile>` excludes `.png`, reconstructs exact `remotePath` (`Documents/a.md`, `b.mdx`), and sorts newest first.
7. `test_download_file_content_enforces_10mib_limit`: mock server returns `10 * 1024 * 1024 + 1` bytes; asserts `Err(DriveApiError::TooLarge(_))`. Normal UTF-8 returns `Ok(content)`.

- [ ] **Step 2: Run tests to verify failure**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml drive_client`
Expected: FAIL.

- [ ] **Step 3: Implement `src-tauri/src/drive_client.rs`**

Implement `DriveClient` HTTP methods, multipart boundary construction (`multipart/related`), Drive JSON response parsing, status-to-`DriveApiError` mapping (`401 -> Unauthorized`, `404 -> NotFound`, `429 -> RateLimited`, `reqwest` connect/timeout -> `Network`), query escaping, RFC 3339 parser, paginated BFS lister (`MAX_LIST_ITEMS = 5000`), and 10 MiB capped streaming/byte-checked download.

- [ ] **Step 4: Run `drive_client` tests and verify pass**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml drive_client`
Expected: PASS (7/7 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/drive_client.rs src-tauri/src/lib.rs
git commit -m "feat(drive): implement Google Drive REST v3 client with 404 healing and tests"
```

---

### Task 4: Sync Engine, Deterministic Path Mapping, & Lifecycle Hooks (`drive_sync.rs`, `fs_ops.rs`, `dialogs.rs`, `history_ops.rs`)

**Files:**
- Create: `src-tauri/src/drive_sync.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod drive_sync;`)
- Modify: `src-tauri/src/commands/fs_ops.rs`
- Modify: `src-tauri/src/commands/dialogs.rs`
- Modify: `src-tauri/src/commands/history_ops.rs`

**Interfaces:**
- Consumes: `HistoryStore`, `DriveAuthManager`, `DriveClient`, `DriveApiError`, `AuthError`.
- Produces in `src-tauri/src/drive_sync.rs`:
  - `pub struct DriveState(pub Option<Arc<DriveManager>>);` (`.manager() -> Result<Arc<DriveManager>, String>`)
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "snake_case")] pub enum DriveSyncState { Disconnected, Paused, Idle, Syncing, Pending, Offline, AuthRequired, Error }` *(note: serialize exact strings `'disconnected' | 'paused' | 'idle' | 'syncing' | 'pending' | 'offline' | 'auth_required' | 'error'` — use explicit `#[serde(rename_all = "snake_case")]` which produces exact matches!)*
  - `pub struct DriveStatus { pub state: DriveSyncState, pub connected: bool, pub account_email: Option<String>, pub root_folder_id: Option<String>, pub settings: DriveSettings, pub pending_count: i64, pub synced_count: i64, pub last_sync_at: Option<i64>, pub last_error: Option<String>, pub auth_in_progress: bool, pub auth_config: DriveAuthConfig }` (`#[serde(rename_all = "camelCase")]`)
  - `pub struct DriveFlushResult { pub completed: bool, pub uploaded_count: usize, pub remaining_count: i64, pub timed_out: bool, pub error: Option<String> }` (`#[serde(rename_all = "camelCase")]`)
  - Functions:
    - `pub fn is_backup_eligible_path(path: &Path) -> bool` (`.md`, `.mdx`, `.markdown` case-insensitive)
    - `pub fn compute_remote_path(local_path: &Path, home_dir: Option<&Path>) -> Result<String, String>`
    - `pub fn compute_backoff_ms(retry_count: i64) -> i64` (`30s, 60s, 120s, 300s, 900s`)
  - `DriveManager` methods:
    - `pub fn new(...) -> Arc<Self>` and `pub fn set_app_handle(&self, app: AppHandle)` (for emitting `"drive:status-changed"` only when wired in Tauri runtime)
    - `pub fn spawn_background_worker(self: &Arc<Self>)`
    - `pub fn get_status(&self) -> Result<DriveStatus, String>`
    - `pub async fn update_settings(&self, settings: DriveSettings) -> Result<DriveStatus, String>`
    - `pub async fn connect_with_opener<F>(&self, workspace_dir: Option<String>, open_file_paths: Vec<String>, open_url: F) -> Result<DriveStatus, AuthError>`
    - `pub fn cancel_auth(&self) -> bool`
    - `pub async fn disconnect(&self) -> Result<DriveStatus, String>`
    - `pub async fn sync_now(&self, workspace_dir: Option<String>, open_file_paths: Vec<String>) -> Result<DriveStatus, String>`
    - `pub async fn enqueue_local_file(&self, path: &Path)`
    - `pub async fn handle_rename(&self, old_path: &Path, new_path: &Path)`
    - `pub async fn flush_before_quit(&self, timeout_ms: u64) -> DriveFlushResult`
    - `pub fn blocking_flush_on_terminate(&self, timeout: std::time::Duration)`
    - `pub async fn list_remote_files(&self) -> Result<Vec<DriveRemoteFile>, String>`
    - `pub async fn download_file_content(&self, file_id: &str) -> Result<String, String>`
    - `pub async fn get_valid_access_token(&self) -> Result<String, String>` (internal helper)

- [ ] **Step 1: Add `pub mod drive_sync;` to `src-tauri/src/lib.rs` and write failing sync engine tests in `src-tauri/src/drive_sync.rs`**

Create `src-tauri/src/drive_sync.rs` with type stubs and these 7 tests:
1. `test_compute_remote_path_home_external_and_traversal`: verifies `/Users/alice/Documents/Notes/a.md` with `home=/Users/alice` -> `"Documents/Notes/a.md"`; `/Volumes/USB/not.md` -> `"_external/Volumes/USB/not.md"`; `/Users/alice/../escape.md` returns `Err`; `photo.png` returns `is_backup_eligible_path == false`.
2. `test_sync_dedup_skips_unchanged_and_moves_on_rename`: seeds connected `MemoryTokenStore` + mock Drive HTTP server tracking request counts. Enqueues `/home/Documents/plan.md` (`"hello"`), runs `sync_now` -> asserts 1 upload. Enqueues identical `"hello"` again -> runs `sync_now` -> asserts 0 extra upload/PATCH requests. Calls `handle_rename` -> `/home/Documents/Proje/plan_yeni.md` (same `"hello"` bytes) and runs `sync_now` -> asserts Drive `PATCH` move/rename was sent (`addParents` + `name`), and `last_synced_remote_path` updated.
3. `test_deleted_local_file_does_not_delete_remote`: enqueues `/home/Documents/gone.md`, syncs it once (`drive_file_id` populated), then deletes `/home/Documents/gone.md` from local disk, enqueues it again, runs sync -> asserts `dirty == false`, NO DELETE request sent to mock server, and `drive_file_id` remains preserved.
4. `test_network_error_backoff_and_invalid_grant_auth_required`: closes/refuses mock server (or returns 503), runs background sweep -> asserts `dirty == true`, `retry_count == 1`, `next_retry_at > now`, and `get_status().state` is `Offline` (connection refused) or `Error` (503). Next, configures token refresh to return `invalid_grant` -> asserts `get_status().state == AuthRequired` and subsequent background worker sweep does not attempt uploads.
5. `test_paused_mode_behavior_and_manual_sync_now`: sets `enabled = false` while connected -> asserts `get_status().state == Paused`; calls `enqueue_local_file` on a new file -> asserts row was NOT added to queue; calls `handle_rename` on previously synced file -> asserts `local_path`/`remote_path` updated cleanly with `dirty = 1`; calls manual `sync_now` -> asserts rename + workspace scan syncs to Drive and state returns to `Paused`.
6. `test_scan_scope_workspace_vs_standalone_tabs`: creates workspace folder `ws/sub/a.md` + outside file `~/Documents/standalone.md` + `~/Documents/untouched.md`; calls `sync_now(None, vec![standalone])` -> asserts `standalone.md` is backed up while `untouched.md` is NOT scanned; calls `sync_now(Some(ws), vec![standalone])` -> asserts both `a.md` and `standalone.md` are backed up while `untouched.md` stays untouched.
7. `test_flush_before_quit_success_and_timeout`: tests (a) fast mock upload finishes before `timeout_ms = 2000` returning `{ completed: true, remainingCount: 0, timedOut: false }`, and (b) intentionally delayed mock server exceeding `timeout_ms = 150` returns `{ completed: false, timedOut: true, remainingCount: 1 }` promptly without hanging.

- [ ] **Step 2: Run tests to verify failure**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml drive_sync`
Expected: FAIL.

- [ ] **Step 3: Implement `src-tauri/src/drive_sync.rs`**

Implement `DriveManager`, `DriveState`, `compute_remote_path` (injecting explicit `home_dir: Option<PathBuf>` inside `DriveManager` so tests never rely on mutating global `$HOME`), `compute_backoff_ms`, 8-state precedence calculation, workspace + standalone tab scanner (`max_depth = 8`, skip `.` except `.cursor`/`.agents`, `node_modules`, `dist`, `dist-electron`, `target`), `sync_lock` queue processor (on `DriveApiError::NotFound`, invalidate cached folder prefix + `clear_drive_file_remote_ids`, re-resolve `ensure_folder_chain`, and retry once; enforce `MAX_BACKUP_BYTES`), background `tokio` loop (`Notify` + idle timer + periodic timer + 2s startup grace), `flush_before_quit`, and `blocking_flush_on_terminate`.

- [ ] **Step 4: Wire best-effort Drive lifecycle hooks into `fs_ops.rs`, `dialogs.rs`, and `history_ops.rs`**

Add optional `Option<Arc<DriveManager>>` parameter to:
- `fs_ops::write_file_inner(..., drive: Option<Arc<DriveManager>>)` & `fs_ops::write_file(..., drive: tauri::State<'_, DriveState>, ...)` -> `drive.enqueue_local_file(path).await`
- `fs_ops::create_file(..., drive: tauri::State<'_, DriveState>)` -> `drive.enqueue_local_file(&target_path).await`
- `fs_ops::rename_file_inner(..., drive: Option<Arc<DriveManager>>)` & `fs_ops::rename_file(..., drive: tauri::State<'_, DriveState>, ...)` -> `drive.handle_rename(Path::new(&old_path), Path::new(&new_path)).await`
- `dialogs::save_file_dialog(..., drive: State<'_, DriveState>)` -> `drive.enqueue_local_file(&path).await`
- `history_ops::restore_snapshot_inner(..., drive: Option<Arc<DriveManager>>)` & `history_ops::history_restore_snapshot(..., drive: State<'_, DriveState>, ...)` -> `drive.enqueue_local_file(target_path).await`
- `history_ops::recover_file_inner(..., drive: Option<Arc<DriveManager>>)` & `history_ops::history_recover_file(..., drive: State<'_, DriveState>, ...)` -> `drive.enqueue_local_file(&dest_path).await`
Keep existing callers in `src-tauri/src/commands/*` tests passing cleanly (`None` for DriveManager in unit tests that only test history/fs).

- [ ] **Step 5: Run all Rust tests across `drive_sync`, `fs_ops`, `history_ops`**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (all tests including `drive_sync`).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/drive_sync.rs src-tauri/src/lib.rs src-tauri/src/commands/fs_ops.rs src-tauri/src/commands/dialogs.rs src-tauri/src/commands/history_ops.rs
git commit -m "feat(drive): add sync engine, path mapping, and best-effort lifecycle hooks"
```

---

### Task 5: Drive IPC Commands, Restore Safety, `Cmd+Q` Quit Interception, & Tauri Wiring

**Files:**
- Create: `src-tauri/src/commands/drive_ops.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/menu.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `DriveManager`, `DriveState`, `fs_ops::validate_single_component`, `fs_ops::resolve_available_path`, `tauri_plugin_opener::OpenerExt`.
- Produces in `src-tauri/src/commands/drive_ops.rs`:
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)] #[serde(rename_all = "camelCase")]`:
    - `DriveRestoreResult { pub file_path: String, pub file_name: String, pub content: String }`
    - `DriveBatchRestoreResult { pub restored_count: usize, pub skipped_count: usize, pub failed_files: Vec<String>, pub target_dir: String }`
  - `pub fn validate_remote_path_segments(remote_path: &str) -> Result<Vec<String>, String>`
  - `pub async fn restore_remote_file_inner(manager: &DriveManager, file_id: &str, remote_path: &str, target_dir: &str, preserve_folders: bool) -> Result<DriveRestoreResult, String>`
  - `pub async fn restore_all_remote_files_inner(manager: &DriveManager, target_dir: &str, preserve_folders: bool) -> Result<DriveBatchRestoreResult, String>`
  - 13 `#[tauri::command]` functions:
    - `drive_get_status(drive: State<'_, DriveState>) -> Result<DriveStatus, String>`
    - `drive_update_settings(drive: State<'_, DriveState>, settings: DriveSettings) -> Result<DriveStatus, String>`
    - `drive_save_custom_oauth(drive: State<'_, DriveState>, client_id: String, client_secret: Option<String>) -> Result<DriveAuthConfig, String>`
    - `drive_clear_custom_oauth(drive: State<'_, DriveState>) -> Result<DriveAuthConfig, String>`
    - `drive_connect(app: AppHandle, drive: State<'_, DriveState>, workspace_dir: Option<String>, open_file_paths: Option<Vec<String>>) -> Result<DriveStatus, String>`
    - `drive_cancel_auth(drive: State<'_, DriveState>) -> Result<bool, String>`
    - `drive_disconnect(drive: State<'_, DriveState>) -> Result<DriveStatus, String>`
    - `drive_sync_now(drive: State<'_, DriveState>, workspace_dir: Option<String>, open_file_paths: Option<Vec<String>>) -> Result<DriveStatus, String>`
    - `drive_flush_before_quit(drive: State<'_, DriveState>, timeout_ms: Option<u64>) -> Result<DriveFlushResult, String>`
    - `drive_list_remote_files(drive: State<'_, DriveState>) -> Result<Vec<DriveRemoteFile>, String>`
    - `drive_download_file_content(drive: State<'_, DriveState>, file_id: String) -> Result<String, String>`
    - `drive_restore_file(drive: State<'_, DriveState>, file_id: String, remote_path: String, target_dir: String, preserve_folders: bool) -> Result<DriveRestoreResult, String>`
    - `drive_restore_all(drive: State<'_, DriveState>, target_dir: String, preserve_folders: bool) -> Result<DriveBatchRestoreResult, String>`

- [ ] **Step 1: Write failing restore safety tests in `src-tauri/src/commands/drive_ops.rs`**

Create `src-tauri/src/commands/drive_ops.rs` with `pub mod drive_ops;` in `src-tauri/src/commands/mod.rs` and 3 tests:
1. `test_drive_restore_file_preserves_folders_and_avoids_collision`: uses `DriveManager` backed by mock HTTP server serving `"# Cloud Note"`; restores `Documents/Work/plan.md` with `preserve_folders = true` into empty `target_dir` -> creates `target_dir/Documents/Work/plan.md`. Calls again -> creates `target_dir/Documents/Work/plan 2.md` without overwriting original. Calls with `preserve_folders = false` -> creates `target_dir/plan.md`.
2. `test_drive_restore_file_rejects_traversal_segments`: passes `remote_path = "Documents/../escape.md"`, `"/abs.md"`, `""`, and `"bad\\name.md"` -> all return `Err` and no file is written outside `target_dir`.
3. `test_drive_restore_all_collects_partial_failures`: mock server lists 2 valid files and 1 file whose download returns 500 -> returns `restored_count == 2`, `failed_files.len() == 1`, and writes the 2 valid files safely.

- [ ] **Step 2: Run tests to verify failure**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml drive_ops`
Expected: FAIL.

- [ ] **Step 3: Implement `src-tauri/src/commands/drive_ops.rs`**

Implement `validate_remote_path_segments`, `restore_remote_file_inner`, `restore_all_remote_files_inner` (using `OpenOptions::create_new(true)` + `resolve_available_path`), and all 13 `#[tauri::command]` wrappers (mapping `AuthError::Cancelled` -> `"AUTH_CANCELLED"`, `AuthError::Timeout` -> `"AUTH_TIMEOUT"`, `AuthError::Denied` -> `"AUTH_DENIED"`, `AuthError::StateMismatch` -> `"AUTH_STATE_MISMATCH"`, `AuthError::NotConfigured` -> `"OAUTH_CLIENT_NOT_CONFIGURED"`). Also attach `register_asset_search_dir` & `allow_asset_dir` on restored files like `history_recover_file`.

- [ ] **Step 4: Update `src-tauri/src/menu.rs` and `src-tauri/src/lib.rs`**

- `menu.rs`:
  - Replace `PredefinedMenuItem::quit` in `app_menu` with `MenuItem::with_id(app, "quit-app", if is_english { "Quit OpenMD" } else { "OpenMD'den Çık" }, true, Some("CmdOrCtrl+Q"))`.
  - Add separator + `"drive-sync-now"` (`"Google Drive'a Şimdi Yedekle"` / `"Back Up to Google Drive Now"`) and `"drive-restore"` (`"Google Drive'dan Geri Yükle..."` / `"Restore from Google Drive..."`) in `file_menu` below `"version-history"`.
  - In `handle_menu_event`, intercept `"quit-app"`: find `"main"` webview window (`app.get_webview_window("main")`), call `unminimize()`, `show()`, `set_focus()`, `close()` (which fires `WindowEvent::CloseRequested`), and `return;` without emitting `"menu:command"`.
- `lib.rs`:
  - Add `pub skip_exit_flush: Arc<AtomicBool>` to `AppState`.
  - Update `close_window(..., app: tauri::AppHandle, force: Option<bool>)`: when `force.unwrap_or(false)`, set `state.force_close = true`, `state.skip_exit_flush = true`, and call `app.exit(0)`.
  - Update `cancel_close(state)`: reset both `state.force_close` and `state.skip_exit_flush` to `false`.
  - Initialize `DriveManager` in `.setup(...)` using the opened `history_store`, `Arc::new(KeychainTokenStore)`, `reqwest::Client`, `option_env!("OPENMD_GOOGLE_CLIENT_ID")`, `option_env!("OPENMD_GOOGLE_CLIENT_SECRET")`, `app.path().home_dir().ok()`, call `manager.set_app_handle(handle.clone())`, `manager.spawn_background_worker()`, and `app.manage(DriveState(drive_manager))`.
  - Register all 13 `drive_ops::*` commands in `invoke_handler!`.
  - Replace `.run(...)` with `.build(tauri::generate_context!())?.run(|app, event| ...)`:
    - `RunEvent::ExitRequested { api, .. }`: if `!app.state::<AppState>().force_close.load(Ordering::SeqCst)`, call `api.prevent_exit()`, show/focus `"main"`, and emit `"app:window-closing"`.
    - `RunEvent::Exit`: if `!app.state::<AppState>().skip_exit_flush.load(Ordering::SeqCst)`, call `manager.blocking_flush_on_terminate(std::time::Duration::from_secs(3))`.

- [ ] **Step 5: Run full Rust test suite and Clippy**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml && /Users/aykuttasdelen/.cargo/bin/cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
Expected: PASS (0 failures, 0 clippy warnings).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/drive_ops.rs src-tauri/src/commands/mod.rs src-tauri/src/menu.rs src-tauri/src/lib.rs
git commit -m "feat(drive): wire Tauri IPC commands, restore safety, and Cmd+Q quit flush"
```

---

### Task 6: Frontend Types, Tauri Bridge, `driveUtils`, & TR/EN Translations

**Files:**
- Modify: `src/types/electron.d.ts`
- Modify: `src/api/tauriBridge.ts`
- Create: `src/utils/driveUtils.ts`
- Modify: `src/i18n/translations.ts`
- Modify: `test/test-utils.js`

**Interfaces:**
- Consumes: Task 5 Tauri command names and camelCase DTO payloads.
- Produces:
  - In `src/types/electron.d.ts`:
    - `DriveSyncState = 'disconnected' | 'paused' | 'idle' | 'syncing' | 'pending' | 'offline' | 'auth_required' | 'error'`
    - `DriveSettings { enabled: boolean; syncIntervalMinutes: number; idleDelaySeconds: number }`
    - `DriveAuthConfig { hasBuiltInClient: boolean; usingCustomClient: boolean; customClientId: string; hasCustomClientSecret: boolean }`
    - `DriveStatus`, `DriveRemoteFile`, `DriveRestoreResult`, `DriveBatchRestoreResult`, `DriveFlushResult`
    - Optional Drive methods on `ElectronAPI` (`driveGetStatus?`, `driveUpdateSettings?`, `driveSaveCustomOAuth?`, `driveClearCustomOAuth?`, `driveConnect?`, `driveCancelAuth?`, `driveDisconnect?`, `driveSyncNow?`, `driveFlushBeforeQuit?`, `driveListRemoteFiles?`, `driveDownloadFileContent?`, `driveRestoreFile?`, `driveRestoreAll?`, `onDriveStatusChanged?`)
  - In `src/utils/driveUtils.ts`:
    - `clampDriveSettings(input: DriveSettings): DriveSettings` (`1..=60` min, `10..=300` sec, rounding finite numbers)
    - `filterRemoteFiles(files: DriveRemoteFile[], query: string): DriveRemoteFile[]` (case-insensitive match on `name` or `remotePath`)
    - `mapDriveErrorCode(error: unknown, t: (key: string, params?: Record<string, string | number>) => string): string` (maps `AUTH_CANCELLED`, `AUTH_TIMEOUT`, `AUTH_DENIED`, `AUTH_STATE_MISMATCH`, `OAUTH_CLIENT_NOT_CONFIGURED` to localized messages, falling back to stringified error)
    - `getDriveStatusTone(state: DriveSyncState): 'muted' | 'warning' | 'success' | 'info' | 'danger'`

- [ ] **Step 1: Add failing `driveUtils` tests to `test/test-utils.js` and add `src/utils/driveUtils.ts` stub**

Append `Test 20: Google Drive Pure Utilities (driveUtils)` in `test/test-utils.js` importing from `../src/utils/driveUtils.ts` and asserting:
- `clampDriveSettings({ enabled: true, syncIntervalMinutes: 0, idleDelaySeconds: 999 })` -> `{ enabled: true, syncIntervalMinutes: 1, idleDelaySeconds: 300 }`
- `clampDriveSettings({ enabled: false, syncIntervalMinutes: 75, idleDelaySeconds: 4.4 })` -> `{ enabled: false, syncIntervalMinutes: 60, idleDelaySeconds: 10 }`
- `filterRemoteFiles` matches both Turkish folder segments and filenames case-insensitively (`'proje'` matches `'Documents/Proje/plan.md'`) and preserves order on empty/whitespace query.
- `mapDriveErrorCode('OAUTH_CLIENT_NOT_CONFIGURED', mockT)` returns mapped translation key rather than raw internal code; `AUTH_CANCELLED` returns mapped key.
- `getDriveStatusTone` maps `'idle' -> 'success'`, `'syncing' -> 'info'`, `'paused' -> 'warning'`, `'pending' -> 'warning'`, `'offline' -> 'warning'`, `'auth_required' -> 'danger'`, `'error' -> 'danger'`, `'disconnected' -> 'muted'`.

- [ ] **Step 2: Run frontend tests to verify failure**

Run: `npm test`
Expected: FAIL.

- [ ] **Step 3: Implement `src/types/electron.d.ts`, `src/api/tauriBridge.ts`, `src/utils/driveUtils.ts`, and `src/i18n/translations.ts`**

- Add all Drive types to `src/types/electron.d.ts` and optional methods on `ElectronAPI`.
- Implement all 13 `invoke` wrappers + `subscribeEvent<DriveStatus>('drive:status-changed', callback)` in `src/api/tauriBridge.ts`.
- Implement pure helpers in `src/utils/driveUtils.ts`.
- Add matching `tr` and `en` translation keys for:
  - Settings Cloud tab (`settings.tabCloud`, `settings.cloudTitle`, `settings.cloudDesc`, `settings.cloudConnect`, `settings.cloudConnecting`, `settings.cloudCancelAuth`, `settings.cloudDisconnect`, `settings.cloudDisconnectConfirm`, `settings.cloudReconnect`, `settings.cloudSyncNow`, `settings.cloudRestore`, `settings.cloudAutoEnabled`, `settings.cloudAutoEnabledDesc`, `settings.cloudIdleDelay`, `settings.cloudIdleDelayDesc`, `settings.cloudSyncInterval`, `settings.cloudSyncIntervalDesc`, `settings.cloudSeconds`, `settings.cloudMinutes`, `settings.cloudStats`, `settings.cloudLastSyncNever`, `settings.cloudScopeNote`, `settings.cloudNotConfiguredBanner`, `settings.cloudByoTitle`, `settings.cloudByoDesc`, `settings.cloudClientId`, `settings.cloudClientSecret`, `settings.cloudClientSecretPlaceholderSaved`, `settings.cloudSaveOAuth`, `settings.cloudClearOAuth`, `settings.cloudSavedOAuth`)
  - StatusBar labels (`status.driveDisconnected`, `status.drivePaused`, `status.driveIdle`, `status.driveSyncing`, `status.drivePending`, `status.driveOffline`, `status.driveAuthRequired`, `status.driveError`, `status.driveTooltip`)
  - DriveRestoreModal (`driveRestore.title`, `driveRestore.searchPlaceholder`, `driveRestore.refresh`, `driveRestore.downloadAll`, `driveRestore.empty`, `driveRestore.selectPrompt`, `driveRestore.loading`, `driveRestore.loadingContent`, `driveRestore.preview`, `driveRestore.openInNewTab`, `driveRestore.restoreToFolder`, `driveRestore.preserveFolders`, `driveRestore.restoredSuccess`, `driveRestore.batchSuccess`, `driveRestore.batchPartialError`, `driveRestore.close`, `driveRestore.fileCount`)
  - DriveQuitSyncModal (`driveQuit.title`, `driveQuit.syncingDesc`, `driveQuit.timeoutTitle`, `driveQuit.errorTitle`, `driveQuit.timeoutDesc`, `driveQuit.offlineDesc`, `driveQuit.skipAndClose`, `driveQuit.retry`, `driveQuit.closeAnyway`, `driveQuit.cancel`)
  - Command Palette & Errors (`command.driveSyncNow`, `command.driveRestore`, `command.driveSettings`, `driveError.authCancelled`, `driveError.authTimeout`, `driveError.authDenied`, `driveError.authStateMismatch`, `driveError.oauthNotConfigured`)

- [ ] **Step 4: Run `npm run typecheck && npm test`**

Run: `npm run typecheck && npm test`
Expected: PASS (0 TS errors; `test-utils.js` passes all tests including i18n TR/EN key parity and `driveUtils`).

- [ ] **Step 5: Commit**

```bash
git add src/types/electron.d.ts src/api/tauriBridge.ts src/utils/driveUtils.ts src/i18n/translations.ts test/test-utils.js
git commit -m "feat(drive): add TypeScript IPC contract, bridge, utils, and TR/EN translations"
```

---

### Task 7: Drive UI Components (`DriveRestoreModal`, `DriveQuitSyncModal`, `SettingsModal`, `StatusBar`)

**Files:**
- Create: `src/components/DriveRestoreModal.tsx`
- Create: `src/components/DriveQuitSyncModal.tsx`
- Modify: `src/components/SettingsModal.tsx`
- Modify: `src/components/StatusBar.tsx`
- Modify: `src/index.css`

**Interfaces:**
- Consumes: `DriveStatus`, `DriveSettings`, `DriveAuthConfig`, `DriveRemoteFile`, `DriveRestoreResult`, `DriveBatchRestoreResult` (`src/types/electron.d.ts`), `driveUtils.ts`, `historyUtils.ts` (`formatByteSize`, `formatRelativeTimestamp`).
- Produces:
  - `DriveRestoreModal`: `{ isOpen: boolean; onClose: () => void; onOpenInNewTab: (fileName: string, content: string) => void; onRestoredToDisk: (result: DriveRestoreResult) => void; onBatchRestored?: (result: DriveBatchRestoreResult) => void }`
    - Guards async calls with `isOpenRef` + monotonic `openSessionRef` + `selectionSeqRef` (matching `HistoryModal` race safety).
    - Disables Escape/overlay close while `isBusy`.
  - `DriveQuitSyncModal`: `{ isOpen: boolean; pendingCount: number; phase: 'syncing' | 'timeout' | 'error'; errorMessage?: string | null; onSkipAndClose: () => void; onRetry: () => void; onCancel: () => void }`
  - `SettingsModal` additions:
    - Optional props: `initialTab?: TabType`, `driveStatus?: DriveStatus | null`, `onDriveConnect?: () => Promise<void>`, `onDriveCancelAuth?: () => Promise<void>`, `onDriveDisconnect?: () => Promise<void>`, `onDriveSyncNow?: () => Promise<void>`, `onDriveUpdateSettings?: (settings: DriveSettings) => Promise<void>`, `onDriveSaveCustomOAuth?: (clientId: string, clientSecret: string | null) => Promise<void>`, `onDriveClearCustomOAuth?: () => Promise<void>`, `onOpenDriveRestore?: () => void`.
    - When `driveStatus` is undefined/null and no Drive handlers are supplied (e.g. web mode), the 5th `'cloud'` tab is NOT rendered.
    - When `initialTab` changes on open, `activeTab` syncs to `initialTab`.
    - When `hasBuiltInClient === false && !usingCustomClient`, automatically expands the BYO accordion and renders a clear setup banner.
    - Never clears `HistoryModal` or breaks existing `files` tab `onMouseDown={e => e.preventDefault()}` behavior.
  - `StatusBar` additions:
    - Optional props: `driveStatus?: DriveStatus | null`, `onOpenDriveSettings?: () => void`.
    - Renders compact clickable pill to the left of `AutoSave` when `driveStatus` is non-null, showing distinct icon + concise localized label for all 8 states (`disconnected`, `paused`, `idle`, `syncing`, `pending`, `offline`, `auth_required`, `error`).

- [ ] **Step 1: Add CSS tokens & component layout styles in `src/index.css`**

Add theme-safe classes for `.status-bar-cloud-btn` (tone variants: `muted`, `success`, `warning`, `info`, `danger`), `.cloud-card`, `.cloud-accordion`, `.drive-restore-modal` split layout, and `.drive-quit-modal`.

- [ ] **Step 2: Create `src/components/DriveRestoreModal.tsx` and `src/components/DriveQuitSyncModal.tsx`**

Implement both modals cleanly using existing modal overlay/palette patterns (`HistoryModal` / `UnsavedChangesModal` conventions).

- [ ] **Step 3: Extend `src/components/SettingsModal.tsx` and `src/components/StatusBar.tsx`**

Add `initialTab` + `'cloud'` tab in `SettingsModal.tsx` (strictly desktop-gated via optional props) and compact clickable Drive pill in `StatusBar.tsx`.

- [ ] **Step 4: Run frontend checks**

Run: `npm run typecheck && npm test && npm run build`
Expected: PASS (0 TS errors, 0 build errors, all `test-utils.js` checks pass).

- [ ] **Step 5: Commit**

```bash
git add src/components/DriveRestoreModal.tsx src/components/DriveQuitSyncModal.tsx src/components/SettingsModal.tsx src/components/StatusBar.tsx src/index.css
git commit -m "feat(drive): add Drive UI components, Cloud settings tab, and StatusBar indicator"
```

---

### Task 8: `App.tsx` Integration, 2-Stage Quit Flush, Menu/Command Palette, & Final Verification

**Files:**
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `window.electronAPI?.drive*`, `DriveRestoreModal`, `DriveQuitSyncModal`, `SettingsModal` (`initialTab`), `StatusBar` (`driveStatus`).
- Produces in `src/App.tsx`:
  - `isDriveAvailable = Boolean(window.electronAPI?.driveGetStatus)`
  - `openFilePaths` helper (`tabs.map(t => t.filePath).filter((p): p is string => Boolean(p))`)
  - Initial `driveGetStatus()` load + `onDriveStatusChanged` subscription + clean reset on unmount
  - Settings integration opening `SettingsModal` on `initialTab` (`'cloud'` when clicking StatusBar badge or `'cmd-drive-settings'`)
  - Menu (`drive-sync-now`, `drive-restore`) & Command Palette actions (gated by `isDriveAvailable`)
  - 2-stage quit flow:
    1. `onWindowClosing` checks `tabs.some(t => t.isModified)`: if unsaved, opens `UnsavedChangesModal` with `pendingCloseAction = 'window'`. Otherwise calls `proceedWithWindowClose()`.
    2. After `handleSaveAllAndClose` (on `'window'`) or `handleDiscardAndClose` (on `'window'`), calls `proceedWithWindowClose()` instead of blindly `closeWindow(true)`.
    3. `proceedWithWindowClose()` checks `window.electronAPI?.driveGetStatus?.()`: if `status?.connected && status.settings.enabled && status.state !== 'auth_required' && status.pendingCount > 0`, opens `DriveQuitSyncModal` and executes `window.electronAPI.driveFlushBeforeQuit(10000)`. On `completed && remainingCount === 0`, closes modal and calls `window.electronAPI.closeWindow(true)`. On failure/timeout, switches modal to `'timeout'` or `'error'`. Otherwise calls `window.electronAPI?.closeWindow(true)` immediately.
    4. Guard against overlapping quit flows with `quitFlushInFlightRef`.
  - Restore handlers:
    - `onOpenInNewTab`: opens editable untitled tab (`filePath: undefined`, `isModified: true`).
    - `onRestoredToDisk` & `onBatchRestored`: opens restored tab when single file restored, and refreshes workspace tree (`handleRefreshTree`) when `rootFolderPath` encloses target directory.

- [ ] **Step 1: Wire `DriveStatus`, handlers, modals, menu commands, Command Palette, and 2-stage close into `src/App.tsx`**

Carefully verify refs/dependencies in `onWindowClosing`, `UnsavedChangesModal` callbacks, `DriveQuitSyncModal` callbacks (`onSkipAndClose` -> `closeWindow(true)`, `onRetry` -> re-run flush, `onCancel` -> close modal & `cancelClose()`), `SettingsModal`, `StatusBar`, and `DriveRestoreModal`. Ensure zero Drive controls or calls run when `isDriveAvailable === false`.

- [ ] **Step 2: Run full frontend & backend verification gates**

Run:
```bash
npm run typecheck && \
npm test && \
npm run build && \
/Users/aykuttasdelen/.cargo/bin/cargo test --manifest-path src-tauri/Cargo.toml && \
/Users/aykuttasdelen/.cargo/bin/cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings && \
git status --short
```
Expected: All 5 toolchains exit 0 with 0 warnings/failures, and `.env.local` stays untracked/ignored (`.env.local` absent from `git status --short`).

- [ ] **Step 3: Verify `build.rs` loads `.env.local` OAuth env vars cleanly during compile**

Run: `/Users/aykuttasdelen/.cargo/bin/cargo check --manifest-path src-tauri/Cargo.toml`
Expected: Exit 0 cleanly with `.env.local` loaded by `src-tauri/build.rs` and zero untracked secret exposure in git.

- [ ] **Step 4: Commit**

```bash
git add src/App.tsx
git commit -m "feat(drive): integrate Google Drive backup, quit flush, and restore flow in App"
```
