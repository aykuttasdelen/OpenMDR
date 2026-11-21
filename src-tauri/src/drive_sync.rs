use crate::drive_auth::{AuthError, DriveAuthManager};
use crate::drive_client::{DriveApiError, DriveClient, DriveRemoteFile, MAX_BACKUP_BYTES};
use crate::history_store::{DriveFileRecord, DriveSettings, HistoryStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex as AsyncMutex, Notify};
use tokio::time::Instant;

const STARTUP_GRACE_DURATION: Duration = Duration::from_secs(2);
const MAX_WORKSPACE_SCAN_DEPTH: usize = 8;

/// Managed Tauri state wrapping the optional Google Drive backup manager.
pub struct DriveState(pub Option<Arc<DriveManager>>);

impl DriveState {
    /// Extracts the initialized `DriveManager` handle or returns a clean error if unavailable.
    pub fn manager(&self) -> Result<Arc<DriveManager>, String> {
        self.0
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| "Google Drive backup manager is unavailable".to_string())
    }
}

/// High-level Google Drive backup state serialized in `snake_case` for the UI status badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveSyncState {
    Disconnected,
    Paused,
    Idle,
    Syncing,
    Pending,
    Offline,
    AuthRequired,
    Error,
}

/// Snapshot of current Google Drive connection, queue, and settings state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveStatus {
    pub state: DriveSyncState,
    pub connected: bool,
    pub account_email: Option<String>,
    pub root_folder_id: Option<String>,
    pub settings: DriveSettings,
    pub pending_count: i64,
    pub synced_count: i64,
    pub last_sync_at: Option<i64>,
    pub last_error: Option<String>,
    pub auth_in_progress: bool,
    pub oauth_configured: bool,
}

/// Result returned after flushing pending Drive uploads before application quit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFlushResult {
    pub completed: bool,
    pub uploaded_count: usize,
    pub remaining_count: i64,
    pub timed_out: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriveErrorKind {
    Offline,
    Error,
}

#[derive(Default)]
struct DriveRuntimeState {
    auth_required: bool,
    last_error_kind: Option<DriveErrorKind>,
    last_error_message: Option<String>,
    idle_deadline: Option<Instant>,
    last_sweep_at: Option<Instant>,
    backoff_until: Option<Instant>,
    worker_started: bool,
}

#[derive(Default)]
struct SweepStats {
    uploaded_count: usize,
    skipped_invalid_count: usize,
    fatal_error: Option<String>,
}

enum RecordSyncOutcome {
    Uploaded,
    SkippedMissing,
    SkippedUnchanged,
    SkippedInvalidLocal(String),
    RetryScheduled {
        kind: DriveErrorKind,
        message: String,
        backoff_ms: i64,
    },
    AuthRequired(String),
}

struct ScannedCandidate {
    local_path: String,
    remote_path: String,
    sha256: String,
}

struct WakeupSnapshot {
    all_dirty: Vec<DriveFileRecord>,
    now_ms: i64,
    now_inst: Instant,
    sync_interval: Duration,
    sync_interval_ms: i64,
}

/// Resets `active_sync` on drop even if an async flush or sync future is cancelled on timeout.
struct ActiveSyncGuard<'a> {
    manager: &'a DriveManager,
    active: bool,
}

impl<'a> ActiveSyncGuard<'a> {
    fn enter(manager: &'a DriveManager) -> Self {
        manager.active_sync.store(true, Ordering::SeqCst);
        manager.emit_status_changed();
        Self {
            manager,
            active: true,
        }
    }

    fn finish(mut self) {
        self.active = false;
        self.manager.active_sync.store(false, Ordering::SeqCst);
    }
}

impl Drop for ActiveSyncGuard<'_> {
    fn drop(&mut self) {
        if self.active {
            self.manager.active_sync.store(false, Ordering::SeqCst);
            self.manager.emit_status_changed();
        }
    }
}

/// Returns `true` when `path` has a case-insensitive `.md`, `.mdx`, or `.markdown` extension.
pub fn is_backup_eligible_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "mdx" | "markdown"))
}

/// Maps a local Markdown path to its deterministic POSIX path beneath Drive's `OpenMD/` folder.
pub fn compute_remote_path(local_path: &Path, home_dir: Option<&Path>) -> Result<String, String> {
    if local_path.as_os_str().is_empty() || local_path.to_string_lossy().contains('\0') {
        return Err("Local path is empty or contains NUL characters".to_string());
    }
    if !is_backup_eligible_path(local_path) {
        return Err("Only .md, .mdx, and .markdown files are eligible for Drive backup".to_string());
    }

    for component in local_path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err("Path cannot contain '.' or '..' traversal segments".to_string());
        }
    }

    let valid_home = home_dir.filter(|home| {
        !home.as_os_str().is_empty()
            && !home
                .components()
                .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
    });

    if let Some(home) = valid_home
        && let Ok(rel) = local_path.strip_prefix(home)
    {
        let segments = collect_normal_segments(rel)?;
        return Ok(segments.join("/"));
    }

    let mut segments = Vec::new();
    for component in local_path.components() {
        match component {
            Component::Prefix(prefix) => {
                let raw = prefix.as_os_str().to_string_lossy();
                let cleaned = raw.trim_matches(['\\', '/']).trim_end_matches(':');
                for part in cleaned.split(['\\', '/']).filter(|s| !s.is_empty()) {
                    segments.push(part.to_string());
                }
            }
            Component::RootDir => {}
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.is_empty() {
                    return Err("Path contains an empty segment".to_string());
                }
                segments.push(text.to_string());
            }
            Component::CurDir | Component::ParentDir => {
                return Err("Path cannot contain '.' or '..' traversal segments".to_string());
            }
        }
    }

    if segments.is_empty() {
        return Err("Remote path must include a file name".to_string());
    }

    Ok(format!("_external/{}", segments.join("/")))
}

/// Returns the exponential retry delay in milliseconds: `30s -> 60s -> 120s -> 300s -> 900s`.
pub fn compute_backoff_ms(retry_count: i64) -> i64 {
    match retry_count {
        ..=1 => 30_000,
        2 => 60_000,
        3 => 120_000,
        4 => 300_000,
        _ => 900_000,
    }
}

/// Coordinates Google Drive authentication, queue persistence, background uploads, and quit flushing.
pub struct DriveManager {
    store: Arc<HistoryStore>,
    auth: Arc<DriveAuthManager>,
    client: DriveClient,
    home_dir: Option<PathBuf>,
    app_handle: StdMutex<Option<AppHandle>>,
    sync_lock: Arc<AsyncMutex<()>>,
    notify: Arc<Notify>,
    active_sync: AtomicBool,
    runtime: StdMutex<DriveRuntimeState>,
}

impl DriveManager {
    /// Creates a new `DriveManager` with an explicit injectable `home_dir` for desktop/test isolation.
    pub fn new(
        store: Arc<HistoryStore>,
        auth: Arc<DriveAuthManager>,
        client: DriveClient,
        home_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            auth,
            client,
            home_dir,
            app_handle: StdMutex::new(None),
            sync_lock: Arc::new(AsyncMutex::new(())),
            notify: Arc::new(Notify::new()),
            active_sync: AtomicBool::new(false),
            runtime: StdMutex::new(DriveRuntimeState::default()),
        })
    }

    /// Alias constructor emphasizing explicit `home_dir` injection.
    pub fn with_home_dir(
        store: Arc<HistoryStore>,
        auth: Arc<DriveAuthManager>,
        client: DriveClient,
        home_dir: Option<PathBuf>,
    ) -> Arc<Self> {
        Self::new(store, auth, client, home_dir)
    }

    /// Binds a Tauri `AppHandle` so state changes emit `"drive:status-changed"` events.
    pub fn set_app_handle(&self, app: AppHandle) {
        if let Ok(mut guard) = self.app_handle.lock() {
            *guard = Some(app);
        }
    }

    /// Starts the single background sync worker loop once with a 2-second startup grace period.
    pub fn spawn_background_worker(self: &Arc<Self>) {
        {
            let Ok(mut rt) = self.runtime.lock() else {
                return;
            };
            if rt.worker_started {
                return;
            }
            rt.worker_started = true;
        }

        let weak_self = Arc::downgrade(self);
        tokio::spawn(async move {
            tokio::time::sleep(STARTUP_GRACE_DURATION).await;

            let Some(mgr) = weak_self.upgrade() else {
                return;
            };
            if mgr.can_background_sync()
                && mgr
                    .store
                    .get_drive_summary()
                    .is_ok_and(|summary| summary.pending_count > 0)
            {
                let _ = mgr.run_background_sweep().await;
            }
            drop(mgr);

            loop {
                let Some(mgr) = weak_self.upgrade() else {
                    break;
                };

                let notify = Arc::clone(&mgr.notify);
                let next_wakeup = mgr.evaluate_and_get_next_wakeup().await;
                drop(mgr);
                let notified = notify.notified();

                match next_wakeup {
                    Some(deadline) => {
                        tokio::select! {
                            _ = notified => {}
                            _ = tokio::time::sleep_until(deadline) => {}
                        }
                    }
                    None => {
                        notified.await;
                    }
                }
            }
        });
    }

    /// Computes the current [`DriveStatus`] according to the 8-state precedence rules.
    pub fn get_status(&self) -> Result<DriveStatus, String> {
        let connected = self.auth.has_stored_tokens();
        let settings = self.store.get_drive_settings()?;
        let summary = self.store.get_drive_summary()?;
        let oauth_configured = self.auth.is_configured();
        let auth_in_progress = self.auth.is_auth_in_progress();

        let account_email = if connected {
            self.store
                .get_drive_setting_str("drive_account_email")?
                .trim()
                .to_string()
                .into_non_empty()
        } else {
            None
        };
        let root_folder_id = if connected {
            self.store
                .get_drive_setting_str("drive_root_folder_id")?
                .trim()
                .to_string()
                .into_non_empty()
        } else {
            None
        };

        let active_sync = self.active_sync.load(Ordering::SeqCst);
        let (auth_required, last_error_kind, last_error) = {
            let guard = self
                .runtime
                .lock()
                .map_err(|e| format!("Drive runtime state poisoned: {e}"))?;
            (
                guard.auth_required,
                guard.last_error_kind,
                guard.last_error_message.clone(),
            )
        };

        let state = if !connected {
            DriveSyncState::Disconnected
        } else if active_sync {
            DriveSyncState::Syncing
        } else if auth_required {
            DriveSyncState::AuthRequired
        } else if !settings.enabled {
            DriveSyncState::Paused
        } else if last_error_kind == Some(DriveErrorKind::Offline) {
            DriveSyncState::Offline
        } else if last_error_kind == Some(DriveErrorKind::Error) {
            DriveSyncState::Error
        } else if summary.pending_count > 0 {
            DriveSyncState::Pending
        } else {
            DriveSyncState::Idle
        };

        Ok(DriveStatus {
            state,
            connected,
            account_email,
            root_folder_id,
            settings,
            pending_count: summary.pending_count,
            synced_count: summary.synced_count,
            last_sync_at: summary.last_sync_at,
            last_error,
            auth_in_progress,
            oauth_configured,
        })
    }

    /// Validates and persists Drive backup settings, waking the background worker if needed.
    pub async fn update_settings(&self, settings: DriveSettings) -> Result<DriveStatus, String> {
        let updated = self.store.update_drive_settings(settings)?;
        if let Ok(mut rt) = self.runtime.lock() {
            if updated.enabled {
                let has_pending = self
                    .store
                    .get_drive_summary()
                    .is_ok_and(|s| s.pending_count > 0);
                if has_pending {
                    rt.idle_deadline = Some(Instant::now() + STARTUP_GRACE_DURATION);
                }
            } else {
                rt.idle_deadline = None;
            }
        }
        self.notify.notify_one();
        self.emit_status_changed();
        self.get_status()
    }

    /// Completes OAuth PKCE sign-in, resolves `OpenMD/`, queues scoped files, and wakes background sync.
    pub async fn connect_with_opener<F>(
        &self,
        workspace_dir: Option<String>,
        open_file_paths: Vec<String>,
        open_url: F,
    ) -> Result<DriveStatus, AuthError>
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        let (_tokens, email) = match self
            .auth
            .bind_loopback_session(|url| {
                self.emit_status_changed();
                open_url(url)
            })
            .await
        {
            Ok(res) => res,
            Err(err) => {
                self.emit_status_changed();
                return Err(err);
            }
        };

        {
            let _sync_guard = self.sync_lock.lock().await;
            let prior_email_raw = self
                .store
                .get_drive_setting_str("drive_account_email")
                .map_err(AuthError::Other)?;
            let prior_email = prior_email_raw.trim();
            let next_email = email.trim();
            if !prior_email.is_empty()
                && (next_email.is_empty() || !prior_email.eq_ignore_ascii_case(next_email))
            {
                self.store
                    .clear_all_drive_cache()
                    .map_err(AuthError::Other)?;
            }

            self.reset_runtime_session_state();

            self.store
                .set_drive_setting_str("drive_account_email", next_email)
                .map_err(AuthError::Other)?;
            let mut settings = self.store.get_drive_settings().map_err(AuthError::Other)?;
            settings.enabled = true;
            self.store
                .update_drive_settings(settings)
                .map_err(AuthError::Other)?;

            let now_ms = current_unix_ms();
            match self.get_valid_access_token_typed().await {
                Ok(access_token) => {
                    match self
                        .client
                        .ensure_root_folder(&access_token, &self.store, now_ms)
                        .await
                    {
                        Ok(_) => {}
                        Err(DriveApiError::Unauthorized(msg)) => {
                            self.mark_auth_required(format!("UNAUTHORIZED: {msg}"));
                        }
                        Err(DriveApiError::Network(msg)) => {
                            self.record_runtime_error(DriveErrorKind::Offline, msg);
                        }
                        Err(err) => {
                            self.record_runtime_error(DriveErrorKind::Error, err.to_string());
                        }
                    }
                }
                Err(AuthError::InvalidGrant(msg)) => {
                    self.mark_auth_required(format!("INVALID_GRANT: {msg}"));
                }
                Err(AuthError::Network(msg)) => {
                    self.record_runtime_error(DriveErrorKind::Offline, msg);
                }
                Err(err) => {
                    self.record_runtime_error(DriveErrorKind::Error, err.to_string());
                }
            }

            if let Err(err) = self
                .scan_and_enqueue_scope(workspace_dir, open_file_paths)
                .await
            {
                eprintln!("[drive] Post-connect scan failed: {err}");
            }
        }

        if let Ok(mut rt) = self.runtime.lock() {
            let has_pending = self
                .store
                .get_drive_summary()
                .is_ok_and(|s| s.pending_count > 0);
            if has_pending {
                rt.idle_deadline = Some(Instant::now());
            }
        }
        self.notify.notify_one();
        self.emit_status_changed();
        self.get_status().map_err(AuthError::Other)
    }

    /// Cancels an active OAuth loopback session if one is waiting.
    pub fn cancel_auth(&self) -> bool {
        let cancelled = self.auth.cancel_auth();
        self.emit_status_changed();
        cancelled
    }

    /// Cancels auth, waits for active sync step, revokes tokens, clears local Drive cache, and emits status.
    pub async fn disconnect(&self) -> Result<DriveStatus, String> {
        self.auth.cancel_auth();
        let _sync_guard = self.sync_lock.lock().await;
        self.auth.revoke_and_clear_tokens().await?;
        self.reset_runtime_session_state();
        self.emit_status_changed();
        self.get_status()
    }

    /// Scans the workspace and/or open tabs and uploads all pending files immediately (even when paused).
    pub async fn sync_now(
        &self,
        workspace_dir: Option<String>,
        open_file_paths: Vec<String>,
    ) -> Result<DriveStatus, String> {
        if !self.auth.has_stored_tokens() {
            return Err("Google Drive is not connected".to_string());
        }

        let _sync_guard = self.sync_lock.lock().await;
        if !self.auth.has_stored_tokens() {
            return Err("Google Drive is not connected".to_string());
        }

        let active_guard = ActiveSyncGuard::enter(self);
        self.scan_and_enqueue_scope(workspace_dir, open_file_paths)
            .await?;

        let stats = self.process_queue_locked(true, None).await?;
        if let Ok(mut rt) = self.runtime.lock() {
            rt.last_sweep_at = Some(Instant::now());
            rt.idle_deadline = None;
            if stats.fatal_error.is_none() && stats.skipped_invalid_count == 0 {
                rt.last_error_kind = None;
                rt.last_error_message = None;
                rt.backoff_until = None;
            }
        }

        active_guard.finish();
        self.emit_status_changed();

        if let Some(err) = stats.fatal_error {
            return Err(err);
        }
        self.get_status()
    }

    /// Enqueues a saved/created/restored local file if Drive is connected, auto-backup is enabled, and path is eligible.
    pub async fn enqueue_local_file(&self, path: &Path) {
        if !self.auth.has_stored_tokens() {
            return;
        }
        let Ok(settings) = self.store.get_drive_settings() else {
            return;
        };
        if !settings.enabled {
            return;
        }
        if !is_backup_eligible_path(path) {
            return;
        }
        let Ok(remote_path) = compute_remote_path(path, self.home_dir.as_deref()) else {
            return;
        };

        let local_str = path.to_string_lossy().to_string();
        let now_ms = current_unix_ms();
        if let Err(err) = self
            .store
            .upsert_drive_file_dirty(&local_str, &remote_path, now_ms)
        {
            eprintln!("[drive] Failed to enqueue {}: {err}", path.display());
            return;
        }

        if let Ok(mut rt) = self.runtime.lock() {
            rt.idle_deadline =
                Some(Instant::now() + Duration::from_secs(u64::from(settings.idle_delay_seconds)));
        }
        self.notify.notify_one();
        self.emit_status_changed();
    }

    /// Updates tracked Drive paths after a local file or folder rename, preserving queue integrity even while paused.
    pub async fn handle_rename(&self, old_path: &Path, new_path: &Path) {
        if !self.auth.has_stored_tokens() {
            return;
        }

        let old_str = old_path.to_string_lossy().to_string();
        let new_str = new_path.to_string_lossy().to_string();
        let home_dir = self.home_dir.clone();
        let now_ms = current_unix_ms();

        let updated_count = match self.store.handle_drive_rename(
            &old_str,
            &new_str,
            |target| compute_remote_path(Path::new(target), home_dir.as_deref()),
            now_ms,
        ) {
            Ok(count) => count,
            Err(err) => {
                eprintln!("[drive] Failed to update Drive path on rename: {err}");
                0
            }
        };

        let settings = match self.store.get_drive_settings() {
            Ok(s) => s,
            Err(_) => return,
        };

        if updated_count > 0 {
            if settings.enabled {
                if let Ok(mut rt) = self.runtime.lock() {
                    rt.idle_deadline = Some(
                        Instant::now() + Duration::from_secs(u64::from(settings.idle_delay_seconds)),
                    );
                }
                self.notify.notify_one();
            }
            self.emit_status_changed();
        } else if settings.enabled && is_backup_eligible_path(new_path) && new_path.is_file() {
            self.enqueue_local_file(new_path).await;
        }
    }

    /// Attempts to flush all pending Drive uploads before quit within `timeout_ms`.
    pub async fn flush_before_quit(&self, timeout_ms: u64) -> DriveFlushResult {
        if !self.auth.has_stored_tokens() || self.is_auth_required() {
            let remaining_count = self
                .store
                .get_drive_summary()
                .map(|s| s.pending_count)
                .unwrap_or(0);
            return DriveFlushResult {
                completed: remaining_count == 0,
                uploaded_count: 0,
                remaining_count,
                timed_out: false,
                error: None,
            };
        }

        if let Ok(settings) = self.store.get_drive_settings()
            && !settings.enabled
        {
            let remaining_count = self
                .store
                .get_drive_summary()
                .map(|s| s.pending_count)
                .unwrap_or(0);
            return DriveFlushResult {
                completed: true,
                uploaded_count: 0,
                remaining_count,
                timed_out: false,
                error: None,
            };
        }

        let initial_pending = self
            .store
            .get_drive_summary()
            .map(|s| s.pending_count)
            .unwrap_or(0);
        if initial_pending <= 0 {
            return DriveFlushResult {
                completed: true,
                uploaded_count: 0,
                remaining_count: 0,
                timed_out: false,
                error: None,
            };
        }

        let uploaded_counter = Arc::new(AtomicUsize::new(0));
        let counter_ref = Arc::clone(&uploaded_counter);

        let flush_fut = async {
            let _sync_guard = self.sync_lock.lock().await;
            let active_guard = ActiveSyncGuard::enter(self);
            let res = self.process_queue_locked(true, Some(&counter_ref)).await;
            if let Ok(stats) = &res
                && stats.fatal_error.is_none()
                && stats.skipped_invalid_count == 0
                && let Ok(mut rt) = self.runtime.lock()
            {
                rt.last_error_kind = None;
                rt.last_error_message = None;
                rt.backoff_until = None;
            }
            active_guard.finish();
            self.emit_status_changed();
            res
        };

        let budget = Duration::from_millis(timeout_ms.max(1));
        match tokio::time::timeout(budget, flush_fut).await {
            Ok(Ok(stats)) => {
                let remaining_count = self
                    .store
                    .get_drive_summary()
                    .map(|s| s.pending_count)
                    .unwrap_or(0);
                let completed = remaining_count == 0 && stats.fatal_error.is_none();
                DriveFlushResult {
                    completed,
                    uploaded_count: uploaded_counter.load(Ordering::SeqCst),
                    remaining_count,
                    timed_out: false,
                    error: stats.fatal_error,
                }
            }
            Ok(Err(err)) => {
                let remaining_count = self
                    .store
                    .get_drive_summary()
                    .map(|s| s.pending_count)
                    .unwrap_or(0);
                DriveFlushResult {
                    completed: false,
                    uploaded_count: uploaded_counter.load(Ordering::SeqCst),
                    remaining_count,
                    timed_out: false,
                    error: Some(err),
                }
            }
            Err(_) => {
                let remaining_count = self
                    .store
                    .get_drive_summary()
                    .map(|s| s.pending_count)
                    .unwrap_or(0);
                DriveFlushResult {
                    completed: false,
                    uploaded_count: uploaded_counter.load(Ordering::SeqCst),
                    remaining_count,
                    timed_out: true,
                    error: Some("Google Drive sync timed out before quit".to_string()),
                }
            }
        }
    }

    /// Best-effort bounded synchronous flush used on abrupt OS termination (`RunEvent::Exit`).
    pub fn blocking_flush_on_terminate(&self, timeout: Duration) {
        if !self.auth.has_stored_tokens() || self.is_auth_required() {
            return;
        }
        if let Ok(settings) = self.store.get_drive_settings()
            && !settings.enabled
        {
            return;
        }
        if let Ok(summary) = self.store.get_drive_summary()
            && summary.pending_count <= 0
        {
            return;
        }

        let timeout_ms = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX).max(1);
        std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let _ = rt.block_on(self.flush_before_quit(timeout_ms));
            });
            let _ = handle.join();
        });
    }

    /// Lists all Markdown/MDX backup files beneath `OpenMD/` on Google Drive.
    pub async fn list_remote_files(&self) -> Result<Vec<DriveRemoteFile>, String> {
        if !self.auth.has_stored_tokens() {
            return Err("Google Drive is not connected".to_string());
        }
        let access_token = self.get_valid_access_token().await?;
        let root_hint = self
            .store
            .get_drive_setting_str("drive_root_folder_id")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        match self
            .client
            .list_files_under_root(&access_token, root_hint.as_deref())
            .await
        {
            Ok(files) => Ok(files),
            Err(DriveApiError::Unauthorized(msg)) => {
                self.mark_auth_required(format!("UNAUTHORIZED: {msg}"));
                self.emit_status_changed();
                Err(format!("UNAUTHORIZED: {msg}"))
            }
            Err(err) => Err(err.to_string()),
        }
    }

    /// Downloads a Drive file's UTF-8 text content (`<= 10 MiB`).
    pub async fn download_file_content(&self, file_id: &str) -> Result<String, String> {
        if !self.auth.has_stored_tokens() {
            return Err("Google Drive is not connected".to_string());
        }
        let access_token = self.get_valid_access_token().await?;
        match self.client.download_file_content(&access_token, file_id).await {
            Ok(content) => Ok(content),
            Err(DriveApiError::Unauthorized(msg)) => {
                self.mark_auth_required(format!("UNAUTHORIZED: {msg}"));
                self.emit_status_changed();
                Err(format!("UNAUTHORIZED: {msg}"))
            }
            Err(err) => Err(err.to_string()),
        }
    }

    /// Returns a valid OAuth access token, setting `AuthRequired` if refresh fails with `invalid_grant`.
    pub async fn get_valid_access_token(&self) -> Result<String, String> {
        self.get_valid_access_token_typed()
            .await
            .map_err(|e| e.to_string())
    }

    /// Executes a single background queue sweep if connected, enabled, and not auth-locked.
    pub(crate) async fn run_background_sweep(&self) -> Result<(), String> {
        if !self.can_background_sync() {
            return Ok(());
        }

        let _sync_guard = self.sync_lock.lock().await;
        if !self.can_background_sync() {
            return Ok(());
        }

        let now_ms = current_unix_ms();
        let due = self.store.list_due_dirty_files(now_ms)?;
        if due.is_empty() {
            if let Ok(mut rt) = self.runtime.lock() {
                rt.last_sweep_at = Some(Instant::now());
            }
            return Ok(());
        }

        let active_guard = ActiveSyncGuard::enter(self);
        let stats = self.process_due_records(&due, None).await?;
        let should_clear_error = stats.fatal_error.is_none()
            && stats.skipped_invalid_count == 0
            && (stats.uploaded_count > 0
                || self
                    .store
                    .get_drive_summary()
                    .is_ok_and(|s| s.pending_count == 0));
        if let Ok(mut rt) = self.runtime.lock() {
            rt.last_sweep_at = Some(Instant::now());
            if should_clear_error {
                rt.last_error_kind = None;
                rt.last_error_message = None;
                rt.backoff_until = None;
            }
        }
        active_guard.finish();
        self.emit_status_changed();
        Ok(())
    }

    async fn get_valid_access_token_typed(&self) -> Result<String, AuthError> {
        match self.auth.get_valid_access_token().await {
            Ok(token) => Ok(token),
            Err(AuthError::InvalidGrant(msg)) => {
                self.mark_auth_required(format!("INVALID_GRANT: {msg}"));
                self.emit_status_changed();
                Err(AuthError::InvalidGrant(msg))
            }
            Err(err) => Err(err),
        }
    }

    async fn evaluate_and_get_next_wakeup(&self) -> Option<Instant> {
        let snap = self.load_wakeup_snapshot()?;
        let (idle_deadline, last_sweep_at, backoff_until) = {
            let Ok(rt) = self.runtime.lock() else {
                return None;
            };
            (rt.idle_deadline, rt.last_sweep_at, rt.backoff_until)
        };

        let due_items: Vec<&DriveFileRecord> = snap
            .all_dirty
            .iter()
            .filter(|r| r.next_retry_at <= snap.now_ms)
            .collect();
        let has_due_items = !due_items.is_empty();
        let backoff_blocked = backoff_until.is_some_and(|b| snap.now_inst < b);
        let retry_due = due_items.iter().any(|r| r.retry_count > 0);
        let idle_due = idle_deadline.is_some_and(|d| snap.now_inst >= d) && has_due_items;
        let earliest_dirty_since_ms = due_items.iter().filter_map(|r| r.dirty_since).min();
        let dirty_interval_elapsed = earliest_dirty_since_ms
            .is_some_and(|since| snap.now_ms.saturating_sub(since) >= snap.sync_interval_ms);
        let sweep_interval_elapsed =
            last_sweep_at.is_none_or(|t| snap.now_inst.duration_since(t) >= snap.sync_interval);
        let periodic_due = has_due_items && dirty_interval_elapsed && sweep_interval_elapsed;

        if !backoff_blocked && (idle_due || periodic_due || retry_due) {
            if let Ok(mut rt) = self.runtime.lock() {
                rt.idle_deadline = None;
            }
            let _ = self.run_background_sweep().await;
            let fresh_snap = self.load_wakeup_snapshot()?;
            return self.compute_next_wakeup_from_snapshot(&fresh_snap);
        }

        self.compute_next_wakeup_from_snapshot(&snap)
    }

    fn load_wakeup_snapshot(&self) -> Option<WakeupSnapshot> {
        if !self.can_background_sync() {
            return None;
        }
        let settings = self.store.get_drive_settings().ok()?;
        let all_dirty = self
            .store
            .list_due_dirty_files(i64::MAX)
            .unwrap_or_default();
        let sync_interval =
            Duration::from_secs(u64::from(settings.sync_interval_minutes.max(1)) * 60);
        let sync_interval_ms = i64::try_from(sync_interval.as_millis()).unwrap_or(i64::MAX);
        Some(WakeupSnapshot {
            all_dirty,
            now_ms: current_unix_ms(),
            now_inst: Instant::now(),
            sync_interval,
            sync_interval_ms,
        })
    }

    fn compute_next_wakeup_from_snapshot(&self, snap: &WakeupSnapshot) -> Option<Instant> {
        let due_items: Vec<&DriveFileRecord> = snap
            .all_dirty
            .iter()
            .filter(|r| r.next_retry_at <= snap.now_ms)
            .collect();
        let has_due_items = !due_items.is_empty();

        let (idle_deadline, last_sweep_at, backoff_until) = {
            let Ok(mut rt) = self.runtime.lock() else {
                return None;
            };
            let backoff_blocked = rt.backoff_until.is_some_and(|b| snap.now_inst < b);
            if rt.idle_deadline.is_some_and(|d| d <= snap.now_inst)
                && (!has_due_items || !backoff_blocked)
            {
                rt.idle_deadline = None;
            }
            (rt.idle_deadline, rt.last_sweep_at, rt.backoff_until)
        };

        let mut next_wakeup = snap.now_inst + snap.sync_interval;

        if let Some(since_ms) = due_items.iter().filter_map(|r| r.dirty_since).min() {
            let dirty_target_ms = since_ms.saturating_add(snap.sync_interval_ms);
            let delta = u64::try_from(dirty_target_ms.saturating_sub(snap.now_ms)).unwrap_or(0);
            let mut periodic_target = snap.now_inst + Duration::from_millis(delta);
            if let Some(last) = last_sweep_at {
                periodic_target = periodic_target.max(last + snap.sync_interval);
            }
            next_wakeup = next_wakeup.min(periodic_target);
        }

        if let Some(d) = idle_deadline {
            next_wakeup = next_wakeup.min(d);
        }
        if let Some(next_retry_ms) = snap
            .all_dirty
            .iter()
            .filter(|r| r.retry_count > 0 && r.next_retry_at > snap.now_ms)
            .map(|r| r.next_retry_at)
            .min()
        {
            let delta = u64::try_from(next_retry_ms.saturating_sub(snap.now_ms)).unwrap_or(0);
            next_wakeup = next_wakeup.min(snap.now_inst + Duration::from_millis(delta));
        }
        if let Some(b) = backoff_until
            && b > snap.now_inst
        {
            next_wakeup = next_wakeup.max(b);
        }

        let min_wakeup = snap.now_inst + Duration::from_millis(50);
        Some(next_wakeup.max(min_wakeup))
    }

    async fn scan_and_enqueue_scope(
        &self,
        workspace_dir: Option<String>,
        open_file_paths: Vec<String>,
    ) -> Result<(), String> {
        let store = Arc::clone(&self.store);
        let home_dir = self.home_dir.clone();
        let now_ms = current_unix_ms();

        tokio::task::spawn_blocking(move || {
            let candidates = collect_scoped_candidates(
                workspace_dir.as_deref(),
                &open_file_paths,
                home_dir.as_deref(),
            );
            for cand in candidates {
                let _ = store.upsert_drive_file_dirty_if_needed(
                    &cand.local_path,
                    &cand.remote_path,
                    &cand.sha256,
                    now_ms,
                );
            }
        })
        .await
        .map_err(|e| format!("Drive workspace scan task failed: {e}"))?;

        Ok(())
    }

    async fn process_queue_locked(
        &self,
        include_future_retries: bool,
        uploaded_counter: Option<&AtomicUsize>,
    ) -> Result<SweepStats, String> {
        let cutoff = if include_future_retries {
            i64::MAX
        } else {
            current_unix_ms()
        };
        let due = self.store.list_due_dirty_files(cutoff)?;
        self.process_due_records(&due, uploaded_counter).await
    }

    async fn process_due_records(
        &self,
        records: &[DriveFileRecord],
        uploaded_counter: Option<&AtomicUsize>,
    ) -> Result<SweepStats, String> {
        let mut stats = SweepStats::default();
        let mut cached_token: Option<String> = None;

        for record in records {
            match self.process_single_record(record, &mut cached_token).await {
                RecordSyncOutcome::Uploaded => {
                    stats.uploaded_count += 1;
                    if let Some(counter) = uploaded_counter {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                }
                RecordSyncOutcome::SkippedMissing | RecordSyncOutcome::SkippedUnchanged => {}
                RecordSyncOutcome::SkippedInvalidLocal(msg) => {
                    stats.skipped_invalid_count += 1;
                    self.record_runtime_error(DriveErrorKind::Error, msg);
                }
                RecordSyncOutcome::RetryScheduled {
                    kind,
                    message,
                    backoff_ms,
                } => {
                    self.record_runtime_error(kind, message.clone());
                    stats.fatal_error = Some(message);
                    if kind == DriveErrorKind::Offline {
                        if let Ok(mut rt) = self.runtime.lock() {
                            rt.backoff_until =
                                Some(Instant::now() + Duration::from_millis(backoff_ms as u64));
                        }
                        break;
                    }
                }
                RecordSyncOutcome::AuthRequired(msg) => {
                    self.mark_auth_required(msg.clone());
                    stats.fatal_error = Some(msg);
                    break;
                }
            }
        }

        Ok(stats)
    }

    async fn process_single_record(
        &self,
        record: &DriveFileRecord,
        cached_token: &mut Option<String>,
    ) -> RecordSyncOutcome {
        let now_ms = current_unix_ms();
        let local_path = Path::new(&record.local_path);

        let bytes = match tokio::fs::read(local_path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                // Local deletion never deletes remote Drive backups.
                let _ = self.store.mark_drive_file_clean(&record.local_path, now_ms);
                return RecordSyncOutcome::SkippedMissing;
            }
            Err(err) => {
                return self.schedule_retry(
                    record,
                    DriveErrorKind::Error,
                    format!("Failed to read {}: {err}", record.local_path),
                    now_ms,
                );
            }
        };

        if bytes.len() > MAX_BACKUP_BYTES {
            let _ = self.store.mark_drive_file_clean(&record.local_path, now_ms);
            return RecordSyncOutcome::SkippedInvalidLocal(format!(
                "Skipped {} ({} bytes exceeds {} byte limit)",
                record.remote_path,
                bytes.len(),
                MAX_BACKUP_BYTES
            ));
        }

        let content = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(err) => {
                let _ = self.store.mark_drive_file_clean(&record.local_path, now_ms);
                return RecordSyncOutcome::SkippedInvalidLocal(format!(
                    "Skipped {} (not valid UTF-8: {err})",
                    record.remote_path
                ));
            }
        };

        let content_hash = compute_sha256_hex(&content);
        if record.last_synced_hash.as_deref() == Some(content_hash.as_str())
            && record.drive_file_id.is_some()
            && record.last_synced_remote_path.as_deref() == Some(record.remote_path.as_str())
        {
            let _ = self.store.mark_drive_file_clean(&record.local_path, now_ms);
            return RecordSyncOutcome::SkippedUnchanged;
        }

        let access_token = match cached_token {
            Some(token) => token.clone(),
            None => match self.get_valid_access_token_typed().await {
                Ok(token) => {
                    *cached_token = Some(token.clone());
                    token
                }
                Err(AuthError::InvalidGrant(msg)) => {
                    return RecordSyncOutcome::AuthRequired(format!("INVALID_GRANT: {msg}"));
                }
                Err(AuthError::Network(msg)) => {
                    return self.schedule_retry(
                        record,
                        DriveErrorKind::Offline,
                        format!("NETWORK_ERROR: {msg}"),
                        now_ms,
                    );
                }
                Err(err) => {
                    return self.schedule_retry(
                        record,
                        DriveErrorKind::Error,
                        err.to_string(),
                        now_ms,
                    );
                }
            },
        };

        let (folder_path, file_name) = split_remote_path(&record.remote_path);
        let folder_segments: Vec<&str> = if folder_path.is_empty() {
            Vec::new()
        } else {
            folder_path.split('/').collect()
        };
        let rename_or_move = record.drive_file_id.is_some()
            && record.last_synced_remote_path.as_deref() != Some(record.remote_path.as_str());

        let first_attempt = self
            .resolve_folders_and_upload(
                &access_token,
                &folder_segments,
                record.drive_file_id.as_deref(),
                record.drive_parent_id.as_deref(),
                file_name,
                &content,
                rename_or_move,
                now_ms,
            )
            .await;

        let upload_res = match first_attempt {
            Err(DriveApiError::NotFound(_)) => {
                // Self-healing retry: clear stale folder prefix and remote file IDs, then retry once.
                let invalidate_target = if folder_path.is_empty() {
                    ""
                } else {
                    folder_path
                };
                let _ = self.store.invalidate_folder_prefix(invalidate_target);
                if invalidate_target.is_empty() {
                    let _ = self.store.set_drive_setting_str("drive_root_folder_id", "");
                }
                let _ = self
                    .store
                    .clear_drive_file_remote_ids(&record.local_path, now_ms);

                let healed_attempt = self
                    .resolve_folders_and_upload(
                        &access_token,
                        &folder_segments,
                        None,
                        None,
                        file_name,
                        &content,
                        false,
                        now_ms,
                    )
                    .await;

                match healed_attempt {
                    Err(DriveApiError::NotFound(_)) if !invalidate_target.is_empty() => {
                        let _ = self.store.invalidate_folder_prefix("");
                        let _ = self.store.set_drive_setting_str("drive_root_folder_id", "");
                        self.resolve_folders_and_upload(
                            &access_token,
                            &folder_segments,
                            None,
                            None,
                            file_name,
                            &content,
                            false,
                            now_ms,
                        )
                        .await
                    }
                    other => other,
                }
            }
            other => other,
        };

        match upload_res {
            Ok(meta) => {
                if let Err(err) = self.store.mark_drive_file_synced(
                    &record.local_path,
                    &record.remote_path,
                    &meta.id,
                    &meta.parent_id,
                    &content_hash,
                    now_ms,
                ) {
                    return self.schedule_retry(
                        record,
                        DriveErrorKind::Error,
                        format!("Failed to persist Drive sync metadata: {err}"),
                        now_ms,
                    );
                }
                RecordSyncOutcome::Uploaded
            }
            Err(DriveApiError::Unauthorized(msg)) => {
                *cached_token = None;
                RecordSyncOutcome::AuthRequired(format!("UNAUTHORIZED: {msg}"))
            }
            Err(DriveApiError::Network(msg)) => {
                self.schedule_retry(record, DriveErrorKind::Offline, msg, now_ms)
            }
            Err(DriveApiError::TooLarge(size)) => {
                let _ = self.store.mark_drive_file_clean(&record.local_path, now_ms);
                RecordSyncOutcome::SkippedInvalidLocal(format!(
                    "File exceeds {MAX_BACKUP_BYTES} byte limit ({size})"
                ))
            }
            Err(err) => {
                self.schedule_retry(record, DriveErrorKind::Error, err.to_string(), now_ms)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn resolve_folders_and_upload(
        &self,
        access_token: &str,
        folder_segments: &[&str],
        existing_file_id: Option<&str>,
        existing_parent_id: Option<&str>,
        file_name: &str,
        content: &str,
        rename_or_move: bool,
        now_ms: i64,
    ) -> Result<crate::drive_client::UploadedFileMeta, DriveApiError> {
        let parent_id = self
            .client
            .ensure_folder_chain(access_token, &self.store, folder_segments, now_ms)
            .await?;
        self.client
            .upload_or_update_file(
                access_token,
                existing_file_id,
                existing_parent_id,
                &parent_id,
                file_name,
                content,
                rename_or_move,
            )
            .await
    }

    fn schedule_retry(
        &self,
        record: &DriveFileRecord,
        kind: DriveErrorKind,
        message: String,
        now_ms: i64,
    ) -> RecordSyncOutcome {
        let next_retry_count = record.retry_count.saturating_add(1);
        let backoff_ms = compute_backoff_ms(next_retry_count);
        let next_retry_at = now_ms.saturating_add(backoff_ms);
        if let Err(err) = self.store.mark_drive_file_retry(
            &record.local_path,
            next_retry_count,
            next_retry_at,
            &message,
            now_ms,
        ) {
            eprintln!("[drive] Failed to persist retry metadata: {err}");
        }
        RecordSyncOutcome::RetryScheduled {
            kind,
            message,
            backoff_ms,
        }
    }

    fn can_background_sync(&self) -> bool {
        if !self.auth.has_stored_tokens() || self.is_auth_required() {
            return false;
        }
        self.store
            .get_drive_settings()
            .is_ok_and(|settings| settings.enabled)
    }

    fn is_auth_required(&self) -> bool {
        self.runtime
            .lock()
            .map(|rt| rt.auth_required)
            .unwrap_or(false)
    }

    fn mark_auth_required(&self, message: String) {
        if let Ok(mut rt) = self.runtime.lock() {
            rt.auth_required = true;
            rt.last_error_kind = None;
            rt.last_error_message = Some(message);
        }
    }

    fn record_runtime_error(&self, kind: DriveErrorKind, message: String) {
        if let Ok(mut rt) = self.runtime.lock() {
            rt.last_error_kind = Some(kind);
            rt.last_error_message = Some(message);
        }
    }

    fn reset_runtime_session_state(&self) {
        if let Ok(mut rt) = self.runtime.lock() {
            rt.auth_required = false;
            rt.last_error_kind = None;
            rt.last_error_message = None;
            rt.idle_deadline = None;
            rt.last_sweep_at = None;
            rt.backoff_until = None;
        }
    }

    fn emit_status_changed(&self) {
        let app = self
            .app_handle
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(app) = app
            && let Ok(status) = self.get_status()
        {
            let _ = app.emit("drive:status-changed", &status);
        }
    }
}

trait StringNonEmptyExt {
    fn into_non_empty(self) -> Option<String>;
}

impl StringNonEmptyExt for String {
    fn into_non_empty(self) -> Option<String> {
        if self.is_empty() { None } else { Some(self) }
    }
}

fn collect_normal_segments(rel: &Path) -> Result<Vec<String>, String> {
    let mut segments = Vec::new();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();
                if text.is_empty() {
                    return Err("Path contains an empty segment".to_string());
                }
                segments.push(text.to_string());
            }
            _ => {
                return Err("Relative path contains invalid segments".to_string());
            }
        }
    }
    if segments.is_empty() {
        return Err("Remote path must include a file name".to_string());
    }
    Ok(segments)
}

fn split_remote_path(remote_path: &str) -> (&str, &str) {
    match remote_path.rsplit_once('/') {
        Some((folder, file)) => (folder, file),
        None => ("", remote_path),
    }
}

fn compute_sha256_hex(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn collect_scoped_candidates(
    workspace_dir: Option<&str>,
    open_file_paths: &[String],
    home_dir: Option<&Path>,
) -> Vec<ScannedCandidate> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    if let Some(ws) = workspace_dir.map(str::trim).filter(|s| !s.is_empty()) {
        walk_workspace_dir(Path::new(ws), 0, home_dir, &mut seen, &mut out);
    }

    for raw_path in open_file_paths {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            continue;
        }
        try_push_candidate(Path::new(trimmed), home_dir, &mut seen, &mut out);
    }

    out
}

fn walk_workspace_dir(
    dir_path: &Path,
    depth: usize,
    home_dir: Option<&Path>,
    seen: &mut HashSet<String>,
    out: &mut Vec<ScannedCandidate>,
) {
    if depth > MAX_WORKSPACE_SCAN_DEPTH {
        return;
    }

    let Ok(entries) = fs::read_dir(dir_path) else {
        return;
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && name != ".cursor" && name != ".agents" {
            continue;
        }
        if matches!(
            name.as_str(),
            "node_modules" | "dist" | "dist-electron" | "target"
        ) {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let full_path = entry.path();
        if file_type.is_dir() {
            walk_workspace_dir(&full_path, depth + 1, home_dir, seen, out);
        } else if file_type.is_file() {
            try_push_candidate(&full_path, home_dir, seen, out);
        }
    }
}

fn try_push_candidate(
    path: &Path,
    home_dir: Option<&Path>,
    seen: &mut HashSet<String>,
    out: &mut Vec<ScannedCandidate>,
) {
    if !is_backup_eligible_path(path) {
        return;
    }
    let norm_local = HistoryStore::normalize_path(&path.to_string_lossy());
    if norm_local.is_empty() || !seen.insert(norm_local.clone()) {
        return;
    }
    let Ok(remote_path) = compute_remote_path(path, home_dir) else {
        return;
    };
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if !meta.is_file() || meta.len() > MAX_BACKUP_BYTES as u64 {
        return;
    }
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    if bytes.len() > MAX_BACKUP_BYTES {
        return;
    }
    let Ok(text) = String::from_utf8(bytes) else {
        return;
    };
    let sha256 = compute_sha256_hex(&text);
    out.push(ScannedCandidate {
        local_path: norm_local,
        remote_path,
        sha256,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive_auth::{MemoryTokenStore, OAuthEndpoints, StoredOAuthTokens, TokenStore};
    use crate::drive_client::{DriveApiEndpoints, DRIVE_FOLDER_MIME};
    use percent_encoding::percent_decode_str;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Debug, Clone)]
    struct MockItem {
        id: String,
        name: String,
        mime_type: String,
        parents: Vec<String>,
        modified_time: String,
        content: Vec<u8>,
        trashed: bool,
    }

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        query: HashMap<String, String>,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    struct MockServerState {
        items: HashMap<String, MockItem>,
        next_id: usize,
        requests: Vec<RecordedRequest>,
        drive_status_override: Option<u16>,
        drive_delay_ms: u64,
        token_status: u16,
        token_body: String,
        token_hits: usize,
        userinfo_email: String,
    }

    impl Default for MockServerState {
        fn default() -> Self {
            Self {
                items: HashMap::new(),
                next_id: 1,
                requests: Vec::new(),
                drive_status_override: None,
                drive_delay_ms: 0,
                token_status: 200,
                token_body: r#"{"access_token":"refreshed-access","refresh_token":"refresh-1","expires_in":3600,"token_type":"Bearer"}"#.to_string(),
                token_hits: 0,
                userinfo_email: "alice@example.com".to_string(),
            }
        }
    }

    impl MockServerState {
        fn alloc_id(&mut self, prefix: &str) -> String {
            let id = format!("{prefix}-{}", self.next_id);
            self.next_id += 1;
            id
        }
    }

    fn parse_query(raw: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for pair in raw.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |s: &str| {
                let plus = s.replace('+', " ");
                percent_decode_str(&plus).decode_utf8_lossy().to_string()
            };
            map.insert(decode(k), decode(v));
        }
        map
    }

    fn extract_quoted_field(q: &str, key_eq: &str) -> Option<String> {
        let start = q.find(key_eq)? + key_eq.len();
        let rest = &q[start..];
        let mut chars = rest.chars();
        if chars.next()? != '\'' {
            return None;
        }
        let mut out = String::new();
        let mut escaped = false;
        for ch in chars {
            if escaped {
                out.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                return Some(out);
            } else {
                out.push(ch);
            }
        }
        None
    }

    fn extract_in_parents(q: &str) -> Option<String> {
        let marker = "' in parents";
        let end = q.find(marker)?;
        let prefix = &q[..end];
        let mut i = prefix.len();
        let bytes = prefix.as_bytes();
        while i > 0 {
            i -= 1;
            if bytes[i] == b'\'' {
                let mut slash_count = 0usize;
                let mut j = i;
                while j > 0 && bytes[j - 1] == b'\\' {
                    slash_count += 1;
                    j -= 1;
                }
                if slash_count.is_multiple_of(2) {
                    let raw = &prefix[i + 1..];
                    return Some(raw.replace("\\'", "'").replace("\\\\", "\\"));
                }
            }
        }
        None
    }

    async fn read_http_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let mut header_end = None;
        let mut content_length = 0usize;
        let mut headers_map = HashMap::new();

        loop {
            let n = stream.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if header_end.is_none()
                && let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n")
            {
                let end = pos + 4;
                header_end = Some(end);
                let headers_str = String::from_utf8_lossy(&buf[..end]);
                for line in headers_str.lines().skip(1) {
                    if let Some((k, v)) = line.split_once(':') {
                        let key = k.trim().to_ascii_lowercase();
                        let val = v.trim().to_string();
                        if key == "content-length" {
                            content_length = val.parse::<usize>().unwrap_or(0);
                        }
                        headers_map.insert(key, val);
                    }
                }
            }
            if let Some(end) = header_end
                && buf.len() >= end + content_length
            {
                break;
            }
        }

        let end = header_end?;
        let headers_str = String::from_utf8_lossy(&buf[..end]);
        let request_line = headers_str.lines().next()?.trim();
        let mut parts = request_line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();
        let (path, query_str) = target.split_once('?').unwrap_or((&target, ""));
        let body_end = (end + content_length).min(buf.len());
        let body = buf[end..body_end].to_vec();

        Some(RecordedRequest {
            method,
            path: path.to_string(),
            query: parse_query(query_str),
            headers: headers_map,
            body,
        })
    }

    fn parse_multipart_related(content_type: &str, body: &[u8]) -> Option<(String, Vec<u8>)> {
        let boundary = content_type
            .split(';')
            .find_map(|part| part.trim().strip_prefix("boundary="))?
            .trim_matches('"');
        let delim = format!("--{boundary}");
        let delim_bytes = delim.as_bytes();
        let first_idx = body
            .windows(delim_bytes.len())
            .position(|w| w == delim_bytes)?;
        let after_first = &body[first_idx + delim_bytes.len()..];
        let after_first_crlf = after_first.strip_prefix(b"\r\n").unwrap_or(after_first);

        let mid_delim = format!("\r\n{delim}").into_bytes();
        let second_rel = after_first_crlf
            .windows(mid_delim.len())
            .position(|w| w == mid_delim.as_slice())?;
        let part1 = &after_first_crlf[..second_rel];
        let part1_hdr_end = part1.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
        let meta_json = String::from_utf8(part1[part1_hdr_end..].to_vec()).ok()?;

        let after_second = &after_first_crlf[second_rel + mid_delim.len()..];
        let after_second_crlf = after_second.strip_prefix(b"\r\n").unwrap_or(after_second);
        let end_delim = format!("\r\n{delim}--").into_bytes();
        let end_rel = after_second_crlf
            .windows(end_delim.len())
            .position(|w| w == end_delim.as_slice())?;
        let part2 = &after_second_crlf[..end_rel];
        let part2_hdr_end = part2.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
        let file_bytes = part2[part2_hdr_end..].to_vec();

        Some((meta_json, file_bytes))
    }

    async fn spawn_mock_google_server(
        state: Arc<Mutex<MockServerState>>,
    ) -> (OAuthEndpoints, DriveApiEndpoints, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let oauth_endpoints = OAuthEndpoints {
            auth_url: format!("{base_url}/auth"),
            token_url: format!("{base_url}/token"),
            revoke_url: format!("{base_url}/revoke"),
            userinfo_url: format!("{base_url}/userinfo"),
        };
        let drive_endpoints = DriveApiEndpoints {
            api_base_url: format!("{base_url}/drive/v3"),
            upload_base_url: format!("{base_url}/upload/drive/v3"),
        };

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let Some(req) = read_http_request(&mut stream).await else {
                        return;
                    };
                    let (delay_ms, status_override) = {
                        let guard = state.lock().unwrap();
                        (guard.drive_delay_ms, guard.drive_status_override)
                    };

                    if req.path == "/token" && req.method == "POST" {
                        let (status, body) = {
                            let mut guard = state.lock().unwrap();
                            guard.token_hits += 1;
                            guard.requests.push(req);
                            (guard.token_status, guard.token_body.clone())
                        };
                        write_json_resp(&mut stream, status, &body).await;
                        return;
                    }
                    if req.path == "/userinfo" && req.method == "GET" {
                        let email = {
                            let mut guard = state.lock().unwrap();
                            guard.requests.push(req);
                            guard.userinfo_email.clone()
                        };
                        write_json_resp(&mut stream, 200, &format!(r#"{{"email":"{email}"}}"#)).await;
                        return;
                    }
                    if req.path == "/revoke" && req.method == "POST" {
                        state.lock().unwrap().requests.push(req);
                        write_json_resp(&mut stream, 200, "{}").await;
                        return;
                    }

                    if delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                    if let Some(code) = status_override {
                        state.lock().unwrap().requests.push(req);
                        write_json_resp(
                            &mut stream,
                            code,
                            &format!(r#"{{"error":{{"message":"Forced status {code}"}}}}"#),
                        )
                        .await;
                        return;
                    }

                    let (status, body) = handle_mock_drive_request(&state, req);
                    write_json_resp(&mut stream, status, &body).await;
                });
            }
        });

        (oauth_endpoints, drive_endpoints, handle)
    }

    async fn write_json_resp(stream: &mut TcpStream, status: u16, body: &str) {
        let status_text = match status {
            200 => "OK",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            503 => "Service Unavailable",
            _ => "Error",
        };
        let resp = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        let _ = stream.flush().await;
    }

    fn handle_mock_drive_request(
        state: &Arc<Mutex<MockServerState>>,
        req: RecordedRequest,
    ) -> (u16, String) {
        let mut guard = state.lock().unwrap();
        guard.requests.push(req.clone());

        if req.method == "GET" && req.path == "/drive/v3/files" {
            let q = req.query.get("q").cloned().unwrap_or_default();
            let name_filter = extract_quoted_field(&q, "name = ");
            let parent_filter = extract_in_parents(&q);
            let want_folder = q.contains(&format!("mimeType = '{DRIVE_FOLDER_MIME}'"));
            let forbid_folder = q.contains(&format!("mimeType != '{DRIVE_FOLDER_MIME}'"));

            let mut matched: Vec<&MockItem> = guard
                .items
                .values()
                .filter(|item| !item.trashed)
                .filter(|item| {
                    if let Some(ref name) = name_filter
                        && item.name != *name
                    {
                        return false;
                    }
                    if let Some(ref parent) = parent_filter
                        && !item.parents.contains(parent)
                    {
                        return false;
                    }
                    if want_folder && item.mime_type != DRIVE_FOLDER_MIME {
                        return false;
                    }
                    if forbid_folder && item.mime_type == DRIVE_FOLDER_MIME {
                        return false;
                    }
                    true
                })
                .collect();
            matched.sort_by(|a, b| a.id.cmp(&b.id));

            let files_json: Vec<serde_json::Value> = matched
                .into_iter()
                .map(|item| {
                    serde_json::json!({
                        "id": item.id,
                        "name": item.name,
                        "mimeType": item.mime_type,
                        "size": item.content.len().to_string(),
                        "modifiedTime": item.modified_time,
                        "parents": item.parents,
                    })
                })
                .collect();
            return (
                200,
                serde_json::json!({ "files": files_json }).to_string(),
            );
        }

        if req.method == "POST" && req.path == "/drive/v3/files" {
            let body_json: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            let name = body_json["name"].as_str().unwrap().to_string();
            let mime_type = body_json["mimeType"]
                .as_str()
                .unwrap_or(DRIVE_FOLDER_MIME)
                .to_string();
            let parents: Vec<String> = body_json["parents"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if let Some(p) = parents.first()
                && p != "root"
                && !guard.items.contains_key(p)
            {
                return (404, r#"{"error":{"message":"Parent not found"}}"#.to_string());
            }
            let id = guard.alloc_id("folder");
            let item = MockItem {
                id: id.clone(),
                name,
                mime_type,
                parents,
                modified_time: "2026-11-21T12:00:00Z".to_string(),
                content: Vec::new(),
                trashed: false,
            };
            guard.items.insert(id, item.clone());
            return (
                200,
                serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "parents": item.parents,
                    "modifiedTime": item.modified_time,
                })
                .to_string(),
            );
        }

        if req.method == "POST" && req.path == "/upload/drive/v3/files" {
            let ct = req.headers.get("content-type").cloned().unwrap_or_default();
            let (meta_str, content) = parse_multipart_related(&ct, &req.body).unwrap();
            let meta: serde_json::Value = serde_json::from_str(&meta_str).unwrap();
            let name = meta["name"].as_str().unwrap().to_string();
            let mime_type = meta["mimeType"]
                .as_str()
                .unwrap_or("text/markdown; charset=utf-8")
                .to_string();
            let parents: Vec<String> = meta["parents"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if let Some(p) = parents.first()
                && p != "root"
                && !guard.items.contains_key(p)
            {
                return (404, r#"{"error":{"message":"Parent not found"}}"#.to_string());
            }
            let id = guard.alloc_id("file");
            let item = MockItem {
                id: id.clone(),
                name,
                mime_type,
                parents,
                modified_time: "2026-11-21T12:00:00Z".to_string(),
                content,
                trashed: false,
            };
            guard.items.insert(id, item.clone());
            return (
                200,
                serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "parents": item.parents,
                    "modifiedTime": item.modified_time,
                })
                .to_string(),
            );
        }

        if req.method == "PATCH" && let Some(file_id) = req.path.strip_prefix("/drive/v3/files/") {
            let Some(item) = guard.items.get_mut(file_id) else {
                return (404, r#"{"error":{"message":"File not found"}}"#.to_string());
            };
            if let Ok(body_json) = serde_json::from_slice::<serde_json::Value>(&req.body)
                && let Some(new_name) = body_json["name"].as_str()
            {
                item.name = new_name.to_string();
            }
            if let Some(remove) = req.query.get("removeParents") {
                for rem in remove.split(',').filter(|s| !s.is_empty()) {
                    item.parents.retain(|p| p != rem);
                }
            }
            if let Some(add) = req.query.get("addParents") {
                for a in add.split(',').filter(|s| !s.is_empty()) {
                    if !item.parents.iter().any(|p| p == a) {
                        item.parents.push(a.to_string());
                    }
                }
            }
            return (
                200,
                serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "parents": item.parents,
                    "modifiedTime": item.modified_time,
                })
                .to_string(),
            );
        }

        if req.method == "PATCH"
            && let Some(file_id) = req.path.strip_prefix("/upload/drive/v3/files/")
        {
            let Some(item) = guard.items.get_mut(file_id) else {
                return (404, r#"{"error":{"message":"File not found"}}"#.to_string());
            };
            item.content = req.body;
            return (
                200,
                serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "parents": item.parents,
                    "modifiedTime": item.modified_time,
                })
                .to_string(),
            );
        }

        if req.method == "GET" && let Some(file_id) = req.path.strip_prefix("/drive/v3/files/") {
            let Some(item) = guard.items.get(file_id) else {
                return (404, r#"{"error":{"message":"File not found"}}"#.to_string());
            };
            if req.query.get("alt").map(String::as_str) == Some("media") {
                return (200, String::from_utf8_lossy(&item.content).to_string());
            }
            return (
                200,
                serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "trashed": item.trashed,
                    "parents": item.parents,
                })
                .to_string(),
            );
        }

        (404, r#"{"error":{"message":"Unhandled mock endpoint"}}"#.to_string())
    }

    fn setup_connected_manager(
        dir: &tempfile::TempDir,
        oauth_endpoints: OAuthEndpoints,
        drive_endpoints: DriveApiEndpoints,
        expires_at_ms: i64,
    ) -> (
        Arc<HistoryStore>,
        Arc<MemoryTokenStore>,
        PathBuf,
        Arc<DriveManager>,
    ) {
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());
        let token_store = Arc::new(MemoryTokenStore::new());
        token_store
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "refresh-1".to_string(),
                access_token: "access-valid".to_string(),
                expires_at_ms,
                email: "alice@example.com".to_string(),
            })
            .unwrap();
        store
            .set_drive_setting_str("drive_account_email", "alice@example.com")
            .unwrap();
        store
            .update_drive_settings(DriveSettings {
                enabled: true,
                sync_interval_minutes: 5,
                idle_delay_seconds: 30,
            })
            .unwrap();

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let auth = Arc::new(
            DriveAuthManager::new(
                Arc::clone(&store),
                token_store.clone(),
                http.clone(),
                oauth_endpoints,
            )
            .with_built_in_credentials(Some("builtin-client".into()), Some("builtin-sec".into())),
        );
        let client = DriveClient::with_endpoints(http, drive_endpoints);
        let home_dir = dir.path().join("home");
        fs::create_dir_all(&home_dir).unwrap();
        let manager = DriveManager::new(store.clone(), auth, client, Some(home_dir.clone()));
        (store, token_store, home_dir, manager)
    }

    fn future_unix_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 3_600_000
    }

    #[test]
    fn test_compute_remote_path_home_external_and_traversal() {
        let home = Path::new("/Users/alice");

        assert!(is_backup_eligible_path(Path::new("a.md")));
        assert!(is_backup_eligible_path(Path::new("a.MDX")));
        assert!(is_backup_eligible_path(Path::new("a.Markdown")));
        assert!(!is_backup_eligible_path(Path::new("photo.png")));
        assert!(!is_backup_eligible_path(Path::new("notes.txt")));

        assert_eq!(
            compute_remote_path(Path::new("/Users/alice/Documents/Notes/a.md"), Some(home))
                .unwrap(),
            "Documents/Notes/a.md"
        );
        assert_eq!(
            compute_remote_path(Path::new("/Volumes/USB/not.md"), Some(home)).unwrap(),
            "_external/Volumes/USB/not.md"
        );
        assert!(
            compute_remote_path(Path::new("/Users/alice/../escape.md"), Some(home)).is_err()
        );
        assert!(compute_remote_path(Path::new("/Users/alice/photo.png"), Some(home)).is_err());
    }

    #[tokio::test]
    async fn test_sync_dedup_skips_unchanged_and_moves_on_rename() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _ts, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();
        let plan_path = docs_dir.join("plan.md");
        fs::write(&plan_path, "hello").unwrap();

        manager.enqueue_local_file(&plan_path).await;
        manager.sync_now(None, vec![]).await.unwrap();

        let upload_count_1 = server_state
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/upload/drive/v3/files")
            .count();
        assert_eq!(upload_count_1, 1);

        let req_count_after_first = server_state.lock().unwrap().requests.len();
        manager.enqueue_local_file(&plan_path).await;
        manager.sync_now(None, vec![]).await.unwrap();
        let req_count_after_second = server_state.lock().unwrap().requests.len();
        assert_eq!(req_count_after_second, req_count_after_first);

        let proje_dir = docs_dir.join("Proje");
        fs::create_dir_all(&proje_dir).unwrap();
        let renamed_path = proje_dir.join("plan_yeni.md");
        fs::rename(&plan_path, &renamed_path).unwrap();

        manager.handle_rename(&plan_path, &renamed_path).await;
        manager.sync_now(None, vec![]).await.unwrap();

        let has_move_patch = server_state.lock().unwrap().requests.iter().any(|r| {
            r.method == "PATCH"
                && r.path.starts_with("/drive/v3/files/")
                && r.query.contains_key("addParents")
                && String::from_utf8_lossy(&r.body).contains("plan_yeni.md")
        });
        assert!(has_move_patch);

        let rec = store
            .get_drive_file(renamed_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            rec.last_synced_remote_path.as_deref(),
            Some("Documents/Proje/plan_yeni.md")
        );
        assert!(!rec.dirty);

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_deleted_local_file_does_not_delete_remote() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _ts, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();
        let gone_path = docs_dir.join("gone.md");
        fs::write(&gone_path, "to be deleted locally").unwrap();

        manager.enqueue_local_file(&gone_path).await;
        manager.sync_now(None, vec![]).await.unwrap();

        let synced_rec = store
            .get_drive_file(gone_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        let saved_file_id = synced_rec.drive_file_id.clone().unwrap();

        fs::remove_file(&gone_path).unwrap();
        manager.enqueue_local_file(&gone_path).await;
        manager.sync_now(None, vec![]).await.unwrap();

        let after_rec = store
            .get_drive_file(gone_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(!after_rec.dirty);
        assert_eq!(after_rec.drive_file_id.as_deref(), Some(saved_file_id.as_str()));
        assert!(!server_state
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r.method == "DELETE"));

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_network_error_backoff_and_invalid_grant_auth_required() {
        let dir = tempdir().unwrap();
        let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed_listener.local_addr().unwrap().port();
        drop(closed_listener);

        let dead_base = format!("http://127.0.0.1:{closed_port}");
        let dead_oauth = OAuthEndpoints {
            auth_url: format!("{dead_base}/auth"),
            token_url: format!("{dead_base}/token"),
            revoke_url: format!("{dead_base}/revoke"),
            userinfo_url: format!("{dead_base}/userinfo"),
        };
        let dead_drive = DriveApiEndpoints {
            api_base_url: format!("{dead_base}/drive/v3"),
            upload_base_url: format!("{dead_base}/upload/drive/v3"),
        };
        let (store, _ts, home_dir, offline_manager) =
            setup_connected_manager(&dir, dead_oauth, dead_drive, future_unix_ms());

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();
        let file_path = docs_dir.join("retry.md");
        fs::write(&file_path, "offline test").unwrap();

        let before_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        offline_manager.enqueue_local_file(&file_path).await;
        let _ = offline_manager.run_background_sweep().await;

        let rec = store
            .get_drive_file(file_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(rec.dirty);
        assert_eq!(rec.retry_count, 1);
        assert!(rec.next_retry_at > before_ms);
        assert_eq!(
            offline_manager.get_status().unwrap().state,
            DriveSyncState::Offline
        );

        let dir2 = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        server_state.lock().unwrap().drive_status_override = Some(503);
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store2, token_store2, home2, err_manager) =
            setup_connected_manager(&dir2, oauth_ep, drive_ep, future_unix_ms());

        let docs2 = home2.join("Documents");
        fs::create_dir_all(&docs2).unwrap();
        let file2 = docs2.join("err.md");
        fs::write(&file2, "503 test").unwrap();

        err_manager.enqueue_local_file(&file2).await;
        let _ = err_manager.run_background_sweep().await;
        let rec2 = store2
            .get_drive_file(file2.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(rec2.dirty);
        assert_eq!(rec2.retry_count, 1);
        assert!(rec2.next_retry_at > 0);
        assert_eq!(
            err_manager.get_status().unwrap().state,
            DriveSyncState::Error
        );

        token_store2
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "revoked-refresh".to_string(),
                access_token: "expired-access".to_string(),
                expires_at_ms: 1,
                email: "alice@example.com".to_string(),
            })
            .unwrap();
        {
            let mut guard = server_state.lock().unwrap();
            guard.drive_status_override = None;
            guard.token_status = 400;
            guard.token_body =
                r#"{"error":"invalid_grant","error_description":"revoked"}"#.to_string();
        }
        err_manager.enqueue_local_file(&file2).await;
        let _ = err_manager.run_background_sweep().await;
        assert_eq!(
            err_manager.get_status().unwrap().state,
            DriveSyncState::AuthRequired
        );

        let reqs_before = server_state.lock().unwrap().requests.len();
        let _ = err_manager.run_background_sweep().await;
        let reqs_after = server_state.lock().unwrap().requests.len();
        assert_eq!(reqs_after, reqs_before);

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_paused_mode_behavior_and_manual_sync_now() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _ts, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();
        let existing_path = docs_dir.join("existing.md");
        fs::write(&existing_path, "synced first").unwrap();
        manager.enqueue_local_file(&existing_path).await;
        manager.sync_now(None, vec![]).await.unwrap();

        manager
            .update_settings(DriveSettings {
                enabled: false,
                sync_interval_minutes: 5,
                idle_delay_seconds: 30,
            })
            .await
            .unwrap();
        assert_eq!(manager.get_status().unwrap().state, DriveSyncState::Paused);

        let new_file = docs_dir.join("paused_new.md");
        fs::write(&new_file, "should not enqueue").unwrap();
        manager.enqueue_local_file(&new_file).await;
        assert!(store
            .get_drive_file(new_file.to_str().unwrap())
            .unwrap()
            .is_none());

        let renamed_existing = docs_dir.join("existing_moved.md");
        fs::rename(&existing_path, &renamed_existing).unwrap();
        manager.handle_rename(&existing_path, &renamed_existing).await;
        let renamed_rec = store
            .get_drive_file(renamed_existing.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(renamed_rec.remote_path, "Documents/existing_moved.md");
        assert!(renamed_rec.dirty);

        let status_after_manual = manager
            .sync_now(Some(docs_dir.to_string_lossy().to_string()), vec![])
            .await
            .unwrap();
        assert_eq!(status_after_manual.state, DriveSyncState::Paused);
        assert_eq!(status_after_manual.pending_count, 0);
        assert!(store
            .get_drive_file(new_file.to_str().unwrap())
            .unwrap()
            .is_some());

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_scan_scope_workspace_vs_standalone_tabs() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _ts, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let ws_sub = home_dir.join("ws").join("sub");
        fs::create_dir_all(&ws_sub).unwrap();
        let ws_file = ws_sub.join("a.md");
        fs::write(&ws_file, "workspace note").unwrap();

        let ws_ignored = home_dir.join("ws").join("node_modules");
        fs::create_dir_all(&ws_ignored).unwrap();
        let ignored_md = ws_ignored.join("ignored.md");
        fs::write(&ignored_md, "should skip").unwrap();

        let ws_cursor = home_dir.join("ws").join(".cursor");
        fs::create_dir_all(&ws_cursor).unwrap();
        let cursor_md = ws_cursor.join("rules.md");
        fs::write(&cursor_md, "cursor rule").unwrap();

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();
        let standalone = docs_dir.join("standalone.md");
        let untouched = docs_dir.join("untouched.md");
        fs::write(&standalone, "open tab").unwrap();
        fs::write(&untouched, "not opened").unwrap();

        manager
            .sync_now(None, vec![standalone.to_string_lossy().to_string()])
            .await
            .unwrap();

        assert!(store
            .get_drive_file(standalone.to_str().unwrap())
            .unwrap()
            .is_some());
        assert!(store
            .get_drive_file(untouched.to_str().unwrap())
            .unwrap()
            .is_none());
        assert!(store
            .get_drive_file(ws_file.to_str().unwrap())
            .unwrap()
            .is_none());

        manager
            .sync_now(
                Some(home_dir.join("ws").to_string_lossy().to_string()),
                vec![standalone.to_string_lossy().to_string()],
            )
            .await
            .unwrap();

        assert!(store
            .get_drive_file(ws_file.to_str().unwrap())
            .unwrap()
            .is_some());
        assert!(store
            .get_drive_file(cursor_md.to_str().unwrap())
            .unwrap()
            .is_some());
        assert!(store
            .get_drive_file(ignored_md.to_str().unwrap())
            .unwrap()
            .is_none());
        assert!(store
            .get_drive_file(standalone.to_str().unwrap())
            .unwrap()
            .is_some());
        assert!(store
            .get_drive_file(untouched.to_str().unwrap())
            .unwrap()
            .is_none());

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_flush_before_quit_success_and_timeout() {
        let dir1 = tempdir().unwrap();
        let state1 = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth1, drive1, handle1) = spawn_mock_google_server(Arc::clone(&state1)).await;
        let (_s1, _ts1, home1, mgr1) =
            setup_connected_manager(&dir1, oauth1, drive1, future_unix_ms());

        let docs1 = home1.join("Documents");
        fs::create_dir_all(&docs1).unwrap();
        let fast_file = docs1.join("fast.md");
        fs::write(&fast_file, "fast flush").unwrap();
        mgr1.enqueue_local_file(&fast_file).await;

        let res1 = mgr1.flush_before_quit(2000).await;
        assert!(res1.completed);
        assert_eq!(res1.remaining_count, 0);
        assert!(!res1.timed_out);
        assert_eq!(res1.uploaded_count, 1);

        let dir2 = tempdir().unwrap();
        let state2 = Arc::new(Mutex::new(MockServerState::default()));
        state2.lock().unwrap().drive_delay_ms = 500;
        let (oauth2, drive2, handle2) = spawn_mock_google_server(Arc::clone(&state2)).await;
        let (_s2, _ts2, home2, mgr2) =
            setup_connected_manager(&dir2, oauth2, drive2, future_unix_ms());

        let docs2 = home2.join("Documents");
        fs::create_dir_all(&docs2).unwrap();
        let slow_file = docs2.join("slow.md");
        fs::write(&slow_file, "slow flush").unwrap();
        mgr2.enqueue_local_file(&slow_file).await;

        let start = std::time::Instant::now();
        let res2 = mgr2.flush_before_quit(150).await;
        let elapsed = start.elapsed();
        assert!(!res2.completed);
        assert!(res2.timed_out);
        assert_eq!(res2.remaining_count, 1);
        assert!(elapsed < Duration::from_millis(450));

        handle1.abort();
        handle2.abort();
    }

    #[tokio::test]
    async fn test_queue_404_self_healing() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _token_store, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let docs_dir = home_dir.join("Documents").join("Notes");
        fs::create_dir_all(&docs_dir).unwrap();
        let note_path = docs_dir.join("heal.md");
        fs::write(&note_path, "v1").unwrap();

        manager.enqueue_local_file(&note_path).await;
        manager.sync_now(None, vec![]).await.unwrap();

        let rec1 = store
            .get_drive_file(note_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        let old_file_id = rec1.drive_file_id.clone().unwrap();

        // Wipe mock server items so cached folders and file ID all return 404 on Drive.
        server_state.lock().unwrap().items.clear();
        fs::write(&note_path, "v2 healed").unwrap();
        manager.enqueue_local_file(&note_path).await;
        let status = manager.sync_now(None, vec![]).await.unwrap();
        assert_eq!(status.state, DriveSyncState::Idle);
        assert_eq!(status.pending_count, 0);

        let rec2 = store
            .get_drive_file(note_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(!rec2.dirty);
        assert_ne!(rec2.drive_file_id.as_deref(), Some(old_file_id.as_str()));

        server_handle.abort();
    }

    #[test]
    fn test_sync_state_serialization_and_backoff_schedule() {
        assert_eq!(
            serde_json::to_string(&DriveSyncState::AuthRequired).unwrap(),
            "\"auth_required\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Disconnected).unwrap(),
            "\"disconnected\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Paused).unwrap(),
            "\"paused\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Syncing).unwrap(),
            "\"syncing\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Offline).unwrap(),
            "\"offline\""
        );
        assert_eq!(
            serde_json::to_string(&DriveSyncState::Error).unwrap(),
            "\"error\""
        );

        assert_eq!(compute_backoff_ms(1), 30_000);
        assert_eq!(compute_backoff_ms(2), 60_000);
        assert_eq!(compute_backoff_ms(3), 120_000);
        assert_eq!(compute_backoff_ms(4), 300_000);
        assert_eq!(compute_backoff_ms(5), 900_000);
        assert_eq!(compute_backoff_ms(9), 900_000);
    }

    #[tokio::test]
    async fn test_connect_list_download_disconnect_and_invalid_local_file() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        {
            let mut guard = server_state.lock().unwrap();
            guard.token_body = r#"{"access_token":"live-access","refresh_token":"live-refresh","expires_in":3600,"token_type":"Bearer"}"#.to_string();
        }
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;

        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());
        let token_store = Arc::new(MemoryTokenStore::new());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let auth = Arc::new(
            DriveAuthManager::new(
                Arc::clone(&store),
                token_store.clone(),
                http.clone(),
                oauth_ep,
            )
            .with_built_in_credentials(Some("builtin-client".into()), Some("builtin-sec".into())),
        );
        let client = DriveClient::with_endpoints(http.clone(), drive_ep);
        let home_dir = dir.path().join("home");
        fs::create_dir_all(home_dir.join("Documents")).unwrap();
        let tab_file = home_dir.join("Documents").join("welcome.md");
        fs::write(&tab_file, "# Welcome").unwrap();

        let manager = DriveManager::new(store.clone(), auth, client, Some(home_dir.clone()));
        assert_eq!(
            manager.get_status().unwrap().state,
            DriveSyncState::Disconnected
        );

        let status = manager
            .connect_with_opener(
                None,
                vec![tab_file.to_string_lossy().to_string()],
                |auth_url| {
                    let parsed = auth_url.split_once('?').unwrap().1;
                    let params = parse_query(parsed);
                    let redirect_uri = params.get("redirect_uri").unwrap().clone();
                    let state = params.get("state").unwrap().clone();
                    let client = reqwest::Client::new();
                    tokio::spawn(async move {
                        let _ = client
                            .get(format!("{redirect_uri}?code=test-code&state={state}"))
                            .send()
                            .await;
                    });
                    Ok(())
                },
            )
            .await
            .unwrap();

        assert!(status.connected);
        assert!(status.settings.enabled);
        assert_eq!(status.account_email.as_deref(), Some("alice@example.com"));
        assert!(status.root_folder_id.is_some());

        manager.sync_now(None, vec![]).await.unwrap();
        let remote_files = manager.list_remote_files().await.unwrap();
        assert_eq!(remote_files.len(), 1);
        assert_eq!(remote_files[0].remote_path, "Documents/welcome.md");

        let downloaded = manager
            .download_file_content(&remote_files[0].file_id)
            .await
            .unwrap();
        assert_eq!(downloaded, "# Welcome");

        // Non-UTF-8 file is marked clean and surfaced via Error status without crashing.
        let bad_utf8 = home_dir.join("Documents").join("binary.md");
        fs::write(&bad_utf8, [0xff_u8, 0xfe, 0x00]).unwrap();
        manager.enqueue_local_file(&bad_utf8).await;
        let err_status = manager.sync_now(None, vec![]).await.unwrap();
        assert_eq!(err_status.state, DriveSyncState::Error);
        assert!(err_status.last_error.is_some());
        assert!(!store
            .get_drive_file(bad_utf8.to_str().unwrap())
            .unwrap()
            .unwrap()
            .dirty);

        let disc_status = manager.disconnect().await.unwrap();
        assert!(!disc_status.connected);
        assert_eq!(disc_status.state, DriveSyncState::Disconnected);
        assert!(token_store.load_tokens().unwrap().is_none());

        server_handle.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_blocking_flush_on_terminate_and_lifecycle_hooks() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _ts, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();

        // Exercise fs_ops + history_ops inner hooks directly.
        let created_path = crate::commands::fs_ops::create_file_inner(
            docs_dir.to_string_lossy().to_string(),
            "hook_note.md".to_string(),
            Some(Arc::clone(&manager)),
        )
        .await
        .unwrap();
        assert!(store.get_drive_file(&created_path).unwrap().unwrap().dirty);

        crate::commands::fs_ops::write_file_inner(
            Some(Arc::clone(&store)),
            created_path.clone(),
            "updated body".to_string(),
            Some("manual".to_string()),
            Some(Arc::clone(&manager)),
        )
        .await
        .unwrap();

        let moved_path = docs_dir.join("hook_note_renamed.md");
        crate::commands::fs_ops::rename_file_inner(
            Some(Arc::clone(&store)),
            created_path.clone(),
            moved_path.to_string_lossy().to_string(),
            Some(Arc::clone(&manager)),
        )
        .await
        .unwrap();
        let moved_rec = store
            .get_drive_file(moved_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(moved_rec.remote_path, "Documents/hook_note_renamed.md");
        assert!(moved_rec.dirty);

        let snaps = store.list_snapshots(moved_path.to_str().unwrap()).unwrap();
        let snap_id = snaps[0].id;
        let file_id = snaps[0].file_id;
        crate::commands::history_ops::restore_snapshot_inner(
            Arc::clone(&store),
            moved_path.to_string_lossy().to_string(),
            snap_id,
            "unsaved before restore".to_string(),
            Some(Arc::clone(&manager)),
        )
        .await
        .unwrap();

        fs::remove_file(&moved_path).unwrap();
        let recovered = crate::commands::history_ops::recover_file_inner(
            Arc::clone(&store),
            file_id,
            snap_id,
            None,
            Some(Arc::clone(&manager)),
        )
        .await
        .unwrap();
        assert!(Path::new(&recovered.file_path).exists());
        assert!(store
            .get_drive_file(&recovered.file_path)
            .unwrap()
            .unwrap()
            .dirty);

        // Run blocking_flush_on_terminate safely even from inside an active Tokio test runtime.
        manager.blocking_flush_on_terminate(Duration::from_secs(2));
        assert_eq!(manager.get_status().unwrap().pending_count, 0);

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_evaluate_wakeup_uses_retry_backoff_after_failed_sweep_and_none_on_auth_required() {
        // (a) 503 Error: post-sweep wakeup must reflect ~30s retry backoff, not 5m sync_interval.
        let dir1 = tempdir().unwrap();
        let state1 = Arc::new(Mutex::new(MockServerState::default()));
        state1.lock().unwrap().drive_status_override = Some(503);
        let (oauth1, drive1, handle1) = spawn_mock_google_server(Arc::clone(&state1)).await;
        let (_store1, token_store1, home1, mgr1) =
            setup_connected_manager(&dir1, oauth1, drive1, future_unix_ms());

        let docs1 = home1.join("Documents");
        fs::create_dir_all(&docs1).unwrap();
        let f1 = docs1.join("err_backoff.md");
        fs::write(&f1, "503 wakeup").unwrap();
        mgr1.enqueue_local_file(&f1).await;
        mgr1.runtime.lock().unwrap().idle_deadline = Some(Instant::now());

        let before1 = Instant::now();
        let wakeup1 = mgr1
            .evaluate_and_get_next_wakeup()
            .await
            .expect("expected scheduled backoff wakeup after 503 error");
        let delta1 = wakeup1.saturating_duration_since(before1);
        assert!(
            delta1 >= Duration::from_secs(20) && delta1 < Duration::from_secs(60),
            "expected ~30s backoff wakeup after 503 sweep, got {delta1:?}"
        );

        // (c) invalid_grant -> AuthRequired: post-sweep wakeup must be None.
        token_store1
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "revoked-refresh".to_string(),
                access_token: "expired-access".to_string(),
                expires_at_ms: 1,
                email: "alice@example.com".to_string(),
            })
            .unwrap();
        {
            let mut guard = state1.lock().unwrap();
            guard.drive_status_override = None;
            guard.token_status = 400;
            guard.token_body =
                r#"{"error":"invalid_grant","error_description":"revoked"}"#.to_string();
        }
        mgr1.enqueue_local_file(&f1).await;
        mgr1.runtime.lock().unwrap().idle_deadline = Some(Instant::now());
        let wakeup_auth = mgr1.evaluate_and_get_next_wakeup().await;
        assert!(
            wakeup_auth.is_none(),
            "expected None wakeup after AuthRequired transition, got {wakeup_auth:?}"
        );
        assert_eq!(mgr1.get_status().unwrap().state, DriveSyncState::AuthRequired);

        handle1.abort();

        // (b) Offline (connection refused): post-sweep wakeup must reflect ~30s backoff, not 5m sync_interval.
        let dir2 = tempdir().unwrap();
        let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed_listener.local_addr().unwrap().port();
        drop(closed_listener);
        let dead_base = format!("http://127.0.0.1:{closed_port}");
        let dead_oauth = OAuthEndpoints {
            auth_url: format!("{dead_base}/auth"),
            token_url: format!("{dead_base}/token"),
            revoke_url: format!("{dead_base}/revoke"),
            userinfo_url: format!("{dead_base}/userinfo"),
        };
        let dead_drive = DriveApiEndpoints {
            api_base_url: format!("{dead_base}/drive/v3"),
            upload_base_url: format!("{dead_base}/upload/drive/v3"),
        };
        let (_store2, _ts2, home2, mgr2) =
            setup_connected_manager(&dir2, dead_oauth, dead_drive, future_unix_ms());
        let docs2 = home2.join("Documents");
        fs::create_dir_all(&docs2).unwrap();
        let f2 = docs2.join("offline_backoff.md");
        fs::write(&f2, "offline wakeup").unwrap();
        mgr2.enqueue_local_file(&f2).await;
        mgr2.runtime.lock().unwrap().idle_deadline = Some(Instant::now());

        let before2 = Instant::now();
        let wakeup2 = mgr2
            .evaluate_and_get_next_wakeup()
            .await
            .expect("expected scheduled backoff wakeup after offline sweep");
        let delta2 = wakeup2.saturating_duration_since(before2);
        assert!(
            delta2 >= Duration::from_secs(20) && delta2 < Duration::from_secs(60),
            "expected ~30s backoff wakeup after offline sweep, got {delta2:?}"
        );
    }

    #[tokio::test]
    async fn test_evaluate_wakeup_clears_stale_idle_deadline_during_error_backoff() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        server_state.lock().unwrap().drive_status_override = Some(503);
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (store, _ts, home_dir, manager) =
            setup_connected_manager(&dir, oauth_ep, drive_ep, future_unix_ms());

        let docs_dir = home_dir.join("Documents");
        fs::create_dir_all(&docs_dir).unwrap();
        let file_path = docs_dir.join("stale_idle.md");
        fs::write(&file_path, "stale idle test").unwrap();

        manager.enqueue_local_file(&file_path).await;
        let _ = manager.run_background_sweep().await;
        let rec = store
            .get_drive_file(file_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        assert!(rec.dirty);
        assert_eq!(rec.retry_count, 1);

        // Simulate a stale past idle_deadline (e.g. from update_settings) while backoff_until is None.
        {
            let mut rt = manager.runtime.lock().unwrap();
            rt.backoff_until = None;
            rt.idle_deadline = Some(Instant::now() - Duration::from_secs(1));
        }

        let before = Instant::now();
        let wakeup = manager
            .evaluate_and_get_next_wakeup()
            .await
            .expect("expected future retry wakeup");
        let delta = wakeup.saturating_duration_since(before);

        assert!(
            manager.runtime.lock().unwrap().idle_deadline.is_none(),
            "stale past idle_deadline must be cleared when no items are due"
        );
        assert!(
            delta >= Duration::from_secs(20) && delta < Duration::from_secs(60),
            "expected future ~30s retry deadline instead of 0ms spin, got {delta:?}"
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_reconnect_different_account_clears_stale_cache_and_uploads_to_new_account() {
        let dir = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        {
            let mut guard = server_state.lock().unwrap();
            guard.userinfo_email = "alice@example.com".to_string();
            guard.token_body = r#"{"access_token":"alice-access","refresh_token":"alice-refresh","expires_in":3600,"token_type":"Bearer"}"#.to_string();
        }
        let (oauth_ep, drive_ep, server_handle) =
            spawn_mock_google_server(Arc::clone(&server_state)).await;

        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());
        let token_store = Arc::new(MemoryTokenStore::new());
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let auth = Arc::new(
            DriveAuthManager::new(
                Arc::clone(&store),
                token_store.clone(),
                http.clone(),
                oauth_ep,
            )
            .with_built_in_credentials(Some("builtin-client".into()), Some("builtin-sec".into())),
        );
        let client = DriveClient::with_endpoints(http, drive_ep);
        let home_dir = dir.path().join("home");
        fs::create_dir_all(home_dir.join("Documents")).unwrap();
        let note_path = home_dir.join("Documents").join("shared.md");
        fs::write(&note_path, "# Unchanged Content").unwrap();
        let note_path_str = note_path.to_string_lossy().to_string();

        let manager = DriveManager::new(store.clone(), auth, client, Some(home_dir));
        let opener = |auth_url: &str| {
            let parsed = auth_url.split_once('?').unwrap().1;
            let params = parse_query(parsed);
            let redirect_uri = params.get("redirect_uri").unwrap().clone();
            let state = params.get("state").unwrap().clone();
            let client = reqwest::Client::new();
            tokio::spawn(async move {
                let _ = client
                    .get(format!("{redirect_uri}?code=test-code&state={state}"))
                    .send()
                    .await;
            });
            Ok(())
        };

        // 1. Alice connects and backs up shared.md.
        let alice_status = manager
            .connect_with_opener(None, vec![note_path_str.clone()], opener)
            .await
            .unwrap();
        assert_eq!(
            alice_status.account_email.as_deref(),
            Some("alice@example.com")
        );
        let alice_root_id = alice_status.root_folder_id.clone().unwrap();

        manager.sync_now(None, vec![]).await.unwrap();
        let alice_rec = store.get_drive_file(&note_path_str).unwrap().unwrap();
        let alice_file_id = alice_rec.drive_file_id.clone().unwrap();
        let alice_docs_folder_id = store
            .get_cached_folder_id("Documents")
            .unwrap()
            .expect("alice Documents folder should be cached");
        assert!(!alice_rec.dirty);
        assert!(alice_rec.last_synced_hash.is_some());

        // 2. Enter auth_required and reconnect using a DIFFERENT Google account (bob@example.com)
        //    whose Drive is completely empty, while shared.md remains unchanged on disk.
        manager.mark_auth_required("INVALID_GRANT: revoked".to_string());
        assert_eq!(
            manager.get_status().unwrap().state,
            DriveSyncState::AuthRequired
        );

        {
            let mut guard = server_state.lock().unwrap();
            guard.items.clear();
            guard.requests.clear();
            guard.userinfo_email = "bob@example.com".to_string();
            guard.token_body = r#"{"access_token":"bob-access","refresh_token":"bob-refresh","expires_in":3600,"token_type":"Bearer"}"#.to_string();
        }

        let bob_status = manager
            .connect_with_opener(None, vec![note_path_str.clone()], opener)
            .await
            .unwrap();
        assert_eq!(bob_status.account_email.as_deref(), Some("bob@example.com"));
        assert_ne!(bob_status.state, DriveSyncState::AuthRequired);
        assert!(bob_status.last_error.is_none());
        assert_ne!(
            bob_status.root_folder_id.as_deref(),
            Some(alice_root_id.as_str()),
            "bob must resolve a fresh OpenMD root folder instead of reusing alice's root ID"
        );
        assert_ne!(
            store.get_cached_folder_id("Documents").unwrap().as_deref(),
            Some(alice_docs_folder_id.as_str()),
            "alice's cached folder IDs must be cleared on account switch"
        );

        let rec_before_bob_sync = store
            .get_drive_file(&note_path_str)
            .unwrap()
            .expect("shared.md should be freshly queued for bob");
        assert!(
            rec_before_bob_sync.dirty,
            "unchanged file must be marked dirty for bob's account"
        );
        assert!(
            rec_before_bob_sync.drive_file_id.is_none(),
            "stale drive_file_id from alice must be cleared"
        );

        // Syncing uploads shared.md into bob's empty Drive.
        let after_sync_status = manager.sync_now(None, vec![]).await.unwrap();
        assert_eq!(after_sync_status.state, DriveSyncState::Idle);
        let bob_remote_files = manager.list_remote_files().await.unwrap();
        assert_eq!(bob_remote_files.len(), 1);
        assert_eq!(bob_remote_files[0].remote_path, "Documents/shared.md");
        assert_ne!(bob_remote_files[0].file_id, alice_file_id);
        assert_eq!(
            manager
                .download_file_content(&bob_remote_files[0].file_id)
                .await
                .unwrap(),
            "# Unchanged Content"
        );

        // 3. Reconnecting with the SAME account (case-insensitive) preserves cache & skips re-upload.
        let bob_file_id = bob_remote_files[0].file_id.clone();
        {
            let mut guard = server_state.lock().unwrap();
            guard.userinfo_email = "Bob@example.com".to_string();
        }
        manager
            .connect_with_opener(None, vec![note_path_str.clone()], opener)
            .await
            .unwrap();
        let rec_same_account = store.get_drive_file(&note_path_str).unwrap().unwrap();
        assert!(!rec_same_account.dirty);
        assert_eq!(
            rec_same_account.drive_file_id.as_deref(),
            Some(bob_file_id.as_str())
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_background_sweep_clears_stale_error_when_pending_drops_to_zero() {
        // (a) Reverting a failing file back to its last_synced_hash clears Error -> Idle.
        let dir1 = tempdir().unwrap();
        let server_state = Arc::new(Mutex::new(MockServerState::default()));
        let (oauth1, drive1, handle1) = spawn_mock_google_server(Arc::clone(&server_state)).await;
        let (_store1, _ts1, home1, mgr1) =
            setup_connected_manager(&dir1, oauth1, drive1, future_unix_ms());

        let docs1 = home1.join("Documents");
        fs::create_dir_all(&docs1).unwrap();
        let revert_file = docs1.join("revert.md");
        fs::write(&revert_file, "v1 clean").unwrap();
        mgr1.enqueue_local_file(&revert_file).await;
        mgr1.sync_now(None, vec![]).await.unwrap();

        server_state.lock().unwrap().drive_status_override = Some(503);
        fs::write(&revert_file, "v2 failing").unwrap();
        mgr1.enqueue_local_file(&revert_file).await;
        let _ = mgr1.run_background_sweep().await;
        assert_eq!(mgr1.get_status().unwrap().state, DriveSyncState::Error);

        // Revert file content to previously synced v1; next sweep has uploaded_count == 0
        // and pending_count == 0, which must clear the stale Error state.
        fs::write(&revert_file, "v1 clean").unwrap();
        mgr1.enqueue_local_file(&revert_file).await;
        mgr1.run_background_sweep().await.unwrap();
        let reverted_status = mgr1.get_status().unwrap();
        assert_eq!(reverted_status.pending_count, 0);
        assert_eq!(reverted_status.state, DriveSyncState::Idle);
        assert!(reverted_status.last_error.is_none());

        // Non-UTF-8 invalid local file still surfaces Error even with pending_count == 0.
        let bad_utf8 = docs1.join("bad_utf8.md");
        fs::write(&bad_utf8, [0xff_u8, 0xfe, 0x00]).unwrap();
        mgr1.enqueue_local_file(&bad_utf8).await;
        mgr1.run_background_sweep().await.unwrap();
        let invalid_status = mgr1.get_status().unwrap();
        assert_eq!(invalid_status.pending_count, 0);
        assert_eq!(invalid_status.state, DriveSyncState::Error);
        assert!(invalid_status.last_error.is_some());

        handle1.abort();

        // (b) Deleting a failing file while Offline clears Offline -> Idle on next sweep.
        let dir2 = tempdir().unwrap();
        let closed_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let closed_port = closed_listener.local_addr().unwrap().port();
        drop(closed_listener);
        let dead_base = format!("http://127.0.0.1:{closed_port}");
        let dead_oauth = OAuthEndpoints {
            auth_url: format!("{dead_base}/auth"),
            token_url: format!("{dead_base}/token"),
            revoke_url: format!("{dead_base}/revoke"),
            userinfo_url: format!("{dead_base}/userinfo"),
        };
        let dead_drive = DriveApiEndpoints {
            api_base_url: format!("{dead_base}/drive/v3"),
            upload_base_url: format!("{dead_base}/upload/drive/v3"),
        };
        let (_store2, _ts2, home2, mgr2) =
            setup_connected_manager(&dir2, dead_oauth, dead_drive, future_unix_ms());

        let docs2 = home2.join("Documents");
        fs::create_dir_all(&docs2).unwrap();
        let deleted_file = docs2.join("deleted_offline.md");
        fs::write(&deleted_file, "offline draft").unwrap();
        mgr2.enqueue_local_file(&deleted_file).await;
        let _ = mgr2.run_background_sweep().await;
        assert_eq!(mgr2.get_status().unwrap().state, DriveSyncState::Offline);

        fs::remove_file(&deleted_file).unwrap();
        mgr2.enqueue_local_file(&deleted_file).await;
        mgr2.run_background_sweep().await.unwrap();
        let deleted_status = mgr2.get_status().unwrap();
        assert_eq!(deleted_status.pending_count, 0);
        assert_eq!(deleted_status.state, DriveSyncState::Idle);
        assert!(deleted_status.last_error.is_none());
    }
}
