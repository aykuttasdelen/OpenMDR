use crate::commands::fs_ops::{resolve_available_path, validate_single_component};
use crate::commands::image_ops::{allow_asset_dir, register_asset_search_dir};
use crate::drive_auth::{AuthError, DriveAuthConfig};
use crate::drive_client::{DriveRemoteFile, MAX_BACKUP_BYTES};
use crate::drive_sync::{DriveFlushResult, DriveManager, DriveState, DriveStatus};
use crate::history_store::DriveSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;
use tokio::io::AsyncWriteExt;

const DEFAULT_QUIT_FLUSH_TIMEOUT_MS: u64 = 10_000;
const MAX_UNIQUE_CREATE_ATTEMPTS: usize = 5;

/// Result returned after restoring a single Google Drive backup file to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveRestoreResult {
    pub file_path: String,
    pub file_name: String,
    pub content: String,
}

/// Summary returned after restoring all Google Drive backup files into a target directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveBatchRestoreResult {
    pub restored_count: usize,
    pub skipped_count: usize,
    pub failed_files: Vec<String>,
    pub target_dir: String,
}

/// Validates every POSIX segment of a remote Drive path to prevent traversal or unsafe file names.
pub fn validate_remote_path_segments(remote_path: &str) -> Result<Vec<String>, String> {
    if remote_path.trim().is_empty() {
        return Err("Remote path cannot be empty".to_string());
    }
    if remote_path.contains('\\') || remote_path.contains('\0') {
        return Err("Remote path contains invalid characters".to_string());
    }

    let mut segments = Vec::new();
    for raw_segment in remote_path.split('/') {
        if raw_segment.trim().is_empty() {
            return Err("Remote path contains an empty segment".to_string());
        }
        validate_single_component(raw_segment)?;
        segments.push(raw_segment.to_string());
    }

    if segments.is_empty() {
        return Err("Remote path must include a file name".to_string());
    }
    Ok(segments)
}

/// Maps structured [`AuthError`] variants into stable frontend IPC error codes.
fn map_auth_error(err: AuthError) -> String {
    match err {
        AuthError::NotConfigured => "OAUTH_CLIENT_NOT_CONFIGURED".to_string(),
        AuthError::Cancelled => "AUTH_CANCELLED".to_string(),
        AuthError::Timeout => "AUTH_TIMEOUT".to_string(),
        AuthError::Denied(_) => "AUTH_DENIED".to_string(),
        AuthError::StateMismatch => "AUTH_STATE_MISMATCH".to_string(),
        other => other.to_string(),
    }
}

async fn write_new_file_collision_safe(
    dest_dir: &Path,
    file_name: &str,
    content: &str,
) -> Result<PathBuf, String> {
    for _ in 0..MAX_UNIQUE_CREATE_ATTEMPTS {
        let dir_for_resolve = dest_dir.to_path_buf();
        let name_for_resolve = file_name.to_string();
        let dest_path = tokio::task::spawn_blocking(move || {
            resolve_available_path(&dir_for_resolve, &name_for_resolve)
        })
        .await
        .map_err(|e| format!("Failed to resolve destination path: {e}"))??;

        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest_path)
            .await
        {
            Ok(mut file) => {
                file.write_all(content.as_bytes())
                    .await
                    .map_err(|e| format!("Failed to write restored file: {e}"))?;
                file.flush()
                    .await
                    .map_err(|e| format!("Failed to flush restored file: {e}"))?;
                return Ok(dest_path);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(format!("Failed to create restored file: {err}")),
        }
    }

    Err("Failed to allocate a unique destination file name".to_string())
}

/// Downloads a remote Drive file and writes it safely to `target_dir` without overwriting existing files.
pub async fn restore_remote_file_inner(
    manager: &DriveManager,
    file_id: &str,
    remote_path: &str,
    target_dir: &str,
    preserve_folders: bool,
) -> Result<DriveRestoreResult, String> {
    if target_dir.trim().is_empty() {
        return Err("Target directory cannot be empty".to_string());
    }
    if file_id.trim().is_empty() {
        return Err("Drive file ID cannot be empty".to_string());
    }

    let segments = validate_remote_path_segments(remote_path)?;
    let (folder_segments, file_name) = (&segments[..segments.len() - 1], &segments[segments.len() - 1]);

    let content = manager.download_file_content(file_id).await?;
    if content.len() > MAX_BACKUP_BYTES {
        return Err(format!(
            "Restored file exceeds {MAX_BACKUP_BYTES} byte limit ({} bytes)",
            content.len()
        ));
    }

    let mut dest_dir = PathBuf::from(target_dir);
    if preserve_folders {
        for seg in folder_segments {
            dest_dir.push(seg);
        }
    }

    tokio::fs::create_dir_all(&dest_dir)
        .await
        .map_err(|e| format!("Failed to create restore directory: {e}"))?;

    let dest_path = write_new_file_collision_safe(&dest_dir, file_name, &content).await?;
    let final_file_name = dest_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| file_name.clone());

    Ok(DriveRestoreResult {
        file_path: dest_path.to_string_lossy().to_string(),
        file_name: final_file_name,
        content,
    })
}

/// Lists all remote Drive files beneath `OpenMD/` and restores them safely into `target_dir`.
pub async fn restore_all_remote_files_inner(
    manager: &DriveManager,
    target_dir: &str,
    preserve_folders: bool,
) -> Result<DriveBatchRestoreResult, String> {
    if target_dir.trim().is_empty() {
        return Err("Target directory cannot be empty".to_string());
    }

    let remote_files = manager.list_remote_files().await?;
    tokio::fs::create_dir_all(target_dir)
        .await
        .map_err(|e| format!("Failed to create restore target directory: {e}"))?;

    let mut restored_count = 0usize;
    let mut skipped_count = 0usize;
    let mut failed_files = Vec::new();

    for file in remote_files {
        if validate_remote_path_segments(&file.remote_path).is_err()
            || file.size_bytes > MAX_BACKUP_BYTES as u64
        {
            skipped_count += 1;
            continue;
        }

        match restore_remote_file_inner(
            manager,
            &file.file_id,
            &file.remote_path,
            target_dir,
            preserve_folders,
        )
        .await
        {
            Ok(_) => restored_count += 1,
            Err(err) => failed_files.push(format!("{}: {err}", file.remote_path)),
        }
    }

    Ok(DriveBatchRestoreResult {
        restored_count,
        skipped_count,
        failed_files,
        target_dir: target_dir.to_string(),
    })
}

/// Returns the current Google Drive connection, queue, and settings status.
#[tauri::command]
pub async fn drive_get_status(drive: State<'_, DriveState>) -> Result<DriveStatus, String> {
    let manager = drive.manager()?;
    tokio::task::spawn_blocking(move || manager.get_status())
        .await
        .map_err(|e| format!("Drive status task failed: {e}"))?
}

/// Validates and persists Google Drive auto-backup settings.
#[tauri::command]
pub async fn drive_update_settings(
    drive: State<'_, DriveState>,
    settings: DriveSettings,
) -> Result<DriveStatus, String> {
    let manager = drive.manager()?;
    manager.update_settings(settings).await
}

/// Saves custom (BYO) OAuth client credentials and clears any stale session if credentials changed.
#[tauri::command]
pub async fn drive_save_custom_oauth(
    drive: State<'_, DriveState>,
    client_id: String,
    client_secret: Option<String>,
) -> Result<DriveAuthConfig, String> {
    let manager = drive.manager()?;
    manager
        .save_custom_oauth(&client_id, client_secret.as_deref())
        .await
}

/// Clears custom (BYO) OAuth client credentials and reverts to the built-in client if configured.
#[tauri::command]
pub async fn drive_clear_custom_oauth(
    drive: State<'_, DriveState>,
) -> Result<DriveAuthConfig, String> {
    let manager = drive.manager()?;
    manager.clear_custom_oauth().await
}

/// Starts Google OAuth 2.0 PKCE loopback sign-in in the default browser and queues scoped workspace/open files.
#[tauri::command]
pub async fn drive_connect(
    app: AppHandle,
    drive: State<'_, DriveState>,
    workspace_dir: Option<String>,
    open_file_paths: Option<Vec<String>>,
) -> Result<DriveStatus, String> {
    let manager = drive.manager()?;
    let open_paths = open_file_paths.unwrap_or_default();
    let opener_app = app.clone();
    manager
        .connect_with_opener(workspace_dir, open_paths, move |url| {
            opener_app
                .opener()
                .open_url(url, None::<&str>)
                .map_err(|e| format!("Failed to open browser for Google sign-in: {e}"))
        })
        .await
        .map_err(map_auth_error)
}

/// Cancels an active in-progress OAuth loopback sign-in session.
#[tauri::command]
pub async fn drive_cancel_auth(drive: State<'_, DriveState>) -> Result<bool, String> {
    let manager = drive.manager()?;
    Ok(manager.cancel_auth())
}

/// Revokes stored Google Drive tokens, clears Keychain and SQLite Drive metadata, and returns updated status.
#[tauri::command]
pub async fn drive_disconnect(drive: State<'_, DriveState>) -> Result<DriveStatus, String> {
    let manager = drive.manager()?;
    manager.disconnect().await
}

/// Scans the active workspace and open tabs and immediately uploads pending Drive backups.
#[tauri::command]
pub async fn drive_sync_now(
    drive: State<'_, DriveState>,
    workspace_dir: Option<String>,
    open_file_paths: Option<Vec<String>>,
) -> Result<DriveStatus, String> {
    let manager = drive.manager()?;
    manager
        .sync_now(workspace_dir, open_file_paths.unwrap_or_default())
        .await
}

/// Flushes pending Google Drive uploads before application quit up to `timeout_ms` (default 10,000 ms).
#[tauri::command]
pub async fn drive_flush_before_quit(
    drive: State<'_, DriveState>,
    timeout_ms: Option<u64>,
) -> Result<DriveFlushResult, String> {
    let manager = drive.manager()?;
    let timeout = timeout_ms.unwrap_or(DEFAULT_QUIT_FLUSH_TIMEOUT_MS);
    Ok(manager.flush_before_quit(timeout).await)
}

/// Lists all Markdown/MDX files stored beneath `OpenMD/` on Google Drive.
#[tauri::command]
pub async fn drive_list_remote_files(
    drive: State<'_, DriveState>,
) -> Result<Vec<DriveRemoteFile>, String> {
    let manager = drive.manager()?;
    manager.list_remote_files().await
}

/// Downloads a remote Google Drive file's UTF-8 text content (`<= 10 MiB`) into memory.
#[tauri::command]
pub async fn drive_download_file_content(
    drive: State<'_, DriveState>,
    file_id: String,
) -> Result<String, String> {
    let manager = drive.manager()?;
    manager.download_file_content(&file_id).await
}

/// Restores a single remote Google Drive file to `target_dir` safely and authorizes its directory for assets.
#[tauri::command]
pub async fn drive_restore_file(
    app: AppHandle,
    drive: State<'_, DriveState>,
    file_id: String,
    remote_path: String,
    target_dir: String,
    preserve_folders: bool,
) -> Result<DriveRestoreResult, String> {
    let manager = drive.manager()?;
    let result = restore_remote_file_inner(
        &manager,
        &file_id,
        &remote_path,
        &target_dir,
        preserve_folders,
    )
    .await?;
    let path = Path::new(&result.file_path);
    register_asset_search_dir(path);
    allow_asset_dir(&app, path);
    Ok(result)
}

/// Restores all remote Google Drive files beneath `OpenMD/` into `target_dir` and authorizes asset loading.
#[tauri::command]
pub async fn drive_restore_all(
    app: AppHandle,
    drive: State<'_, DriveState>,
    target_dir: String,
    preserve_folders: bool,
) -> Result<DriveBatchRestoreResult, String> {
    let manager = drive.manager()?;
    let result = restore_all_remote_files_inner(&manager, &target_dir, preserve_folders).await?;
    if result.restored_count > 0 {
        let dir_path = Path::new(&result.target_dir);
        register_asset_search_dir(dir_path);
        allow_asset_dir(&app, dir_path);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive_auth::{
        DriveAuthManager, MemoryTokenStore, OAuthEndpoints, StoredOAuthTokens, TokenStore,
    };
    use crate::drive_client::{DriveApiEndpoints, DriveClient, DRIVE_FOLDER_MIME};
    use crate::history_store::{DriveSettings, HistoryStore};
    use percent_encoding::percent_decode_str;
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use tempfile::tempdir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Clone)]
    struct MockDriveEntry {
        id: String,
        name: String,
        mime_type: String,
        parents: Vec<String>,
        content: Vec<u8>,
        fail_download_status: Option<u16>,
    }

    struct MockServerState {
        items: HashMap<String, MockDriveEntry>,
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

    fn extract_in_parents(q: &str) -> Option<String> {
        let marker = "' in parents";
        let end = q.find(marker)?;
        let prefix = &q[..end];
        let start = prefix.rfind('\'')? + 1;
        Some(prefix[start..].to_string())
    }

    async fn read_request(stream: &mut TcpStream) -> Option<(String, String, HashMap<String, String>)> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 2048];
        let header_end = loop {
            let n = stream.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers_str = String::from_utf8_lossy(&buf[..header_end]);
        let request_line = headers_str.lines().next()?.trim();
        let mut parts = request_line.split_whitespace();
        let method = parts.next()?.to_string();
        let target = parts.next()?.to_string();
        let (path, query_str) = target.split_once('?').unwrap_or((&target, ""));
        Some((method, path.to_string(), parse_query(query_str)))
    }

    async fn write_http_response(
        stream: &mut TcpStream,
        status: u16,
        content_type: &str,
        body: &[u8],
    ) {
        let status_text = match status {
            200 => "OK",
            404 => "Not Found",
            500 => "Internal Server Error",
            _ => "Error",
        };
        let header = format!(
            "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes()).await;
        let _ = stream.write_all(body).await;
        let _ = stream.flush().await;
    }

    async fn spawn_mock_drive_server(
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
                    let Some((method, path, query)) = read_request(&mut stream).await else {
                        return;
                    };
                    if method == "GET" && path == "/drive/v3/files" {
                        let q = query.get("q").cloned().unwrap_or_default();
                        let parent_filter = extract_in_parents(&q);
                        let want_folder = q.contains(&format!("mimeType = '{DRIVE_FOLDER_MIME}'"));
                        let files_json: Vec<serde_json::Value> = {
                            let guard = state.lock().unwrap();
                            let mut items: Vec<&MockDriveEntry> = guard
                                .items
                                .values()
                                .filter(|item| {
                                    if let Some(ref parent) = parent_filter
                                        && !item.parents.contains(parent)
                                    {
                                        return false;
                                    }
                                    !(want_folder && item.mime_type != DRIVE_FOLDER_MIME)
                                })
                                .collect();
                            items.sort_by(|a, b| a.id.cmp(&b.id));
                            items
                                .into_iter()
                                .map(|item| {
                                    serde_json::json!({
                                        "id": item.id,
                                        "name": item.name,
                                        "mimeType": item.mime_type,
                                        "size": item.content.len().to_string(),
                                        "modifiedTime": "2026-11-21T12:00:00Z",
                                        "parents": item.parents,
                                    })
                                })
                                .collect()
                        };
                        let payload = serde_json::json!({ "files": files_json }).to_string();
                        write_http_response(&mut stream, 200, "application/json", payload.as_bytes())
                            .await;
                        return;
                    }

                    if method == "GET"
                        && let Some(file_id) = path.strip_prefix("/drive/v3/files/")
                    {
                        enum LookupResult {
                            Meta(serde_json::Value),
                            Media(Vec<u8>),
                            Error(u16, String),
                        }
                        let result = {
                            let guard = state.lock().unwrap();
                            match guard.items.get(file_id) {
                                None => LookupResult::Error(
                                    404,
                                    r#"{"error":{"message":"Not found"}}"#.to_string(),
                                ),
                                Some(item) => {
                                    if query.get("alt").map(String::as_str) == Some("media") {
                                        if let Some(status) = item.fail_download_status {
                                            LookupResult::Error(
                                                status,
                                                format!(
                                                    r#"{{"error":{{"message":"Download error {status}"}}}}"#
                                                ),
                                            )
                                        } else {
                                            LookupResult::Media(item.content.clone())
                                        }
                                    } else {
                                        LookupResult::Meta(serde_json::json!({
                                            "id": item.id,
                                            "name": item.name,
                                            "trashed": false,
                                            "parents": item.parents,
                                        }))
                                    }
                                }
                            }
                        };
                        match result {
                            LookupResult::Meta(val) => {
                                let body = val.to_string();
                                write_http_response(
                                    &mut stream,
                                    200,
                                    "application/json",
                                    body.as_bytes(),
                                )
                                .await;
                            }
                            LookupResult::Media(bytes) => {
                                write_http_response(
                                    &mut stream,
                                    200,
                                    "text/markdown; charset=utf-8",
                                    &bytes,
                                )
                                .await;
                            }
                            LookupResult::Error(status, body) => {
                                write_http_response(
                                    &mut stream,
                                    status,
                                    "application/json",
                                    body.as_bytes(),
                                )
                                .await;
                            }
                        }
                        return;
                    }

                    write_http_response(&mut stream, 404, "application/json", b"{}").await;
                });
            }
        });

        (oauth_endpoints, drive_endpoints, handle)
    }

    fn setup_manager(
        dir: &tempfile::TempDir,
        oauth_endpoints: OAuthEndpoints,
        drive_endpoints: DriveApiEndpoints,
    ) -> (Arc<DriveManager>, PathBuf) {
        let db_path = dir.path().join("history.db");
        let store = Arc::new(HistoryStore::open(db_path).unwrap());
        let token_store = Arc::new(MemoryTokenStore::new());
        let future_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
            + 3_600_000;
        token_store
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "refresh-1".to_string(),
                access_token: "access-valid".to_string(),
                expires_at_ms: future_ms,
                email: "alice@example.com".to_string(),
            })
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
                token_store,
                http.clone(),
                oauth_endpoints,
            )
            .with_built_in_credentials(Some("client-id".into()), Some("client-secret".into())),
        );
        let client = DriveClient::with_endpoints(http, drive_endpoints);
        let home_dir = dir.path().join("home");
        fs::create_dir_all(&home_dir).unwrap();
        let manager = DriveManager::new(store, auth, client, Some(home_dir));
        let target_dir = dir.path().join("restore_target");
        fs::create_dir_all(&target_dir).unwrap();
        (manager, target_dir)
    }

    #[tokio::test]
    async fn test_drive_restore_file_preserves_folders_and_avoids_collision() {
        let dir = tempdir().unwrap();
        let mut items = HashMap::new();
        items.insert(
            "file-1".to_string(),
            MockDriveEntry {
                id: "file-1".to_string(),
                name: "plan.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["work-folder".to_string()],
                content: b"# Cloud Note".to_vec(),
                fail_download_status: None,
            },
        );
        let server_state = Arc::new(Mutex::new(MockServerState { items }));
        let (oauth_ep, drive_ep, server_handle) = spawn_mock_drive_server(server_state).await;
        let (manager, target_dir) = setup_manager(&dir, oauth_ep, drive_ep);
        let target_str = target_dir.to_string_lossy().to_string();

        let res1 = restore_remote_file_inner(
            &manager,
            "file-1",
            "Documents/Work/plan.md",
            &target_str,
            true,
        )
        .await
        .unwrap();

        let expected1 = target_dir.join("Documents").join("Work").join("plan.md");
        assert_eq!(PathBuf::from(&res1.file_path), expected1);
        assert_eq!(res1.file_name, "plan.md");
        assert_eq!(res1.content, "# Cloud Note");
        assert_eq!(fs::read_to_string(&expected1).unwrap(), "# Cloud Note");

        fs::write(&expected1, "# Existing Local").unwrap();

        let res2 = restore_remote_file_inner(
            &manager,
            "file-1",
            "Documents/Work/plan.md",
            &target_str,
            true,
        )
        .await
        .unwrap();

        let expected2 = target_dir.join("Documents").join("Work").join("plan 2.md");
        assert_eq!(PathBuf::from(&res2.file_path), expected2);
        assert_eq!(res2.file_name, "plan 2.md");
        assert_eq!(fs::read_to_string(&expected2).unwrap(), "# Cloud Note");
        assert_eq!(fs::read_to_string(&expected1).unwrap(), "# Existing Local");

        let res3 = restore_remote_file_inner(
            &manager,
            "file-1",
            "Documents/Work/plan.md",
            &target_str,
            false,
        )
        .await
        .unwrap();

        let expected3 = target_dir.join("plan.md");
        assert_eq!(PathBuf::from(&res3.file_path), expected3);
        assert_eq!(res3.file_name, "plan.md");
        assert_eq!(fs::read_to_string(&expected3).unwrap(), "# Cloud Note");

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_drive_restore_file_rejects_traversal_segments() {
        let dir = tempdir().unwrap();
        let mut items = HashMap::new();
        items.insert(
            "file-1".to_string(),
            MockDriveEntry {
                id: "file-1".to_string(),
                name: "escape.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["root".to_string()],
                content: b"evil".to_vec(),
                fail_download_status: None,
            },
        );
        let server_state = Arc::new(Mutex::new(MockServerState { items }));
        let (oauth_ep, drive_ep, server_handle) = spawn_mock_drive_server(server_state).await;
        let (manager, target_dir) = setup_manager(&dir, oauth_ep, drive_ep);
        let target_str = target_dir.to_string_lossy().to_string();

        assert_eq!(
            validate_remote_path_segments("Documents/Work/plan.md").unwrap(),
            vec!["Documents", "Work", "plan.md"]
        );

        for bad_path in [
            "Documents/../escape.md",
            "/abs.md",
            "",
            "bad\\name.md",
            "Documents/./escape.md",
            "bad\0name.md",
        ] {
            assert!(
                restore_remote_file_inner(&manager, "file-1", bad_path, &target_str, true)
                    .await
                    .is_err(),
                "expected traversal/invalid remote path `{bad_path}` to be rejected"
            );
        }

        assert!(!dir.path().join("escape.md").exists());
        assert!(!dir.path().join("abs.md").exists());
        assert!(!target_dir.join("escape.md").exists());
        assert!(!target_dir.join("abs.md").exists());

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_drive_restore_all_collects_partial_failures() {
        let dir = tempdir().unwrap();
        let mut items = HashMap::new();
        items.insert(
            "root-openmd".to_string(),
            MockDriveEntry {
                id: "root-openmd".to_string(),
                name: "OpenMD".to_string(),
                mime_type: DRIVE_FOLDER_MIME.to_string(),
                parents: vec!["root".to_string()],
                content: Vec::new(),
                fail_download_status: None,
            },
        );
        items.insert(
            "folder-work".to_string(),
            MockDriveEntry {
                id: "folder-work".to_string(),
                name: "Work".to_string(),
                mime_type: DRIVE_FOLDER_MIME.to_string(),
                parents: vec!["root-openmd".to_string()],
                content: Vec::new(),
                fail_download_status: None,
            },
        );
        items.insert(
            "file-1".to_string(),
            MockDriveEntry {
                id: "file-1".to_string(),
                name: "readme.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["root-openmd".to_string()],
                content: b"# Root Readme".to_vec(),
                fail_download_status: None,
            },
        );
        items.insert(
            "file-2".to_string(),
            MockDriveEntry {
                id: "file-2".to_string(),
                name: "spec.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["folder-work".to_string()],
                content: b"# Work Spec".to_vec(),
                fail_download_status: None,
            },
        );
        items.insert(
            "file-3".to_string(),
            MockDriveEntry {
                id: "file-3".to_string(),
                name: "broken.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["folder-work".to_string()],
                content: b"unreachable".to_vec(),
                fail_download_status: Some(500),
            },
        );

        let server_state = Arc::new(Mutex::new(MockServerState { items }));
        let (oauth_ep, drive_ep, server_handle) = spawn_mock_drive_server(server_state).await;
        let (manager, target_dir) = setup_manager(&dir, oauth_ep, drive_ep);
        let target_str = target_dir.to_string_lossy().to_string();

        let res = restore_all_remote_files_inner(&manager, &target_str, true)
            .await
            .unwrap();

        assert_eq!(res.restored_count, 2);
        assert_eq!(res.skipped_count, 0);
        assert_eq!(res.failed_files.len(), 1);
        assert_eq!(res.target_dir, target_str);
        assert_eq!(
            fs::read_to_string(target_dir.join("readme.md")).unwrap(),
            "# Root Readme"
        );
        assert_eq!(
            fs::read_to_string(target_dir.join("Work").join("spec.md")).unwrap(),
            "# Work Spec"
        );
        assert!(!target_dir.join("Work").join("broken.md").exists());

        server_handle.abort();
    }

    #[test]
    fn test_auth_error_mapping_and_camel_case_serialization() {
        assert_eq!(map_auth_error(AuthError::Cancelled), "AUTH_CANCELLED");
        assert_eq!(map_auth_error(AuthError::Timeout), "AUTH_TIMEOUT");
        assert_eq!(
            map_auth_error(AuthError::Denied("access_denied".to_string())),
            "AUTH_DENIED"
        );
        assert_eq!(
            map_auth_error(AuthError::StateMismatch),
            "AUTH_STATE_MISMATCH"
        );
        assert_eq!(
            map_auth_error(AuthError::NotConfigured),
            "OAUTH_CLIENT_NOT_CONFIGURED"
        );

        let single = serde_json::to_value(DriveRestoreResult {
            file_path: "/tmp/plan.md".to_string(),
            file_name: "plan.md".to_string(),
            content: "# Hi".to_string(),
        })
        .unwrap();
        assert_eq!(single["filePath"], "/tmp/plan.md");
        assert_eq!(single["fileName"], "plan.md");
        assert_eq!(single["content"], "# Hi");

        let batch = serde_json::to_value(DriveBatchRestoreResult {
            restored_count: 2,
            skipped_count: 1,
            failed_files: vec!["bad.md".to_string()],
            target_dir: "/tmp".to_string(),
        })
        .unwrap();
        assert_eq!(batch["restoredCount"], 2);
        assert_eq!(batch["skippedCount"], 1);
        assert_eq!(batch["failedFiles"], serde_json::json!(["bad.md"]));
        assert_eq!(batch["targetDir"], "/tmp");
    }

    #[tokio::test]
    async fn test_drive_restore_enforces_size_limit_and_skips_unsafe_batch_entries() {
        let dir = tempdir().unwrap();
        let mut items = HashMap::new();
        items.insert(
            "root-openmd".to_string(),
            MockDriveEntry {
                id: "root-openmd".to_string(),
                name: "OpenMD".to_string(),
                mime_type: DRIVE_FOLDER_MIME.to_string(),
                parents: vec!["root".to_string()],
                content: Vec::new(),
                fail_download_status: None,
            },
        );
        items.insert(
            "folder-traversal".to_string(),
            MockDriveEntry {
                id: "folder-traversal".to_string(),
                name: "..".to_string(),
                mime_type: DRIVE_FOLDER_MIME.to_string(),
                parents: vec!["root-openmd".to_string()],
                content: Vec::new(),
                fail_download_status: None,
            },
        );
        items.insert(
            "file-evil".to_string(),
            MockDriveEntry {
                id: "file-evil".to_string(),
                name: "escape.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["folder-traversal".to_string()],
                content: b"evil".to_vec(),
                fail_download_status: None,
            },
        );
        items.insert(
            "file-huge".to_string(),
            MockDriveEntry {
                id: "file-huge".to_string(),
                name: "huge.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["root-openmd".to_string()],
                content: vec![b'a'; MAX_BACKUP_BYTES + 1],
                fail_download_status: None,
            },
        );
        items.insert(
            "file-ok".to_string(),
            MockDriveEntry {
                id: "file-ok".to_string(),
                name: "ok.md".to_string(),
                mime_type: "text/markdown; charset=utf-8".to_string(),
                parents: vec!["root-openmd".to_string()],
                content: b"# OK".to_vec(),
                fail_download_status: None,
            },
        );

        let server_state = Arc::new(Mutex::new(MockServerState { items }));
        let (oauth_ep, drive_ep, server_handle) = spawn_mock_drive_server(server_state).await;
        let (manager, target_dir) = setup_manager(&dir, oauth_ep, drive_ep);
        let target_str = target_dir.to_string_lossy().to_string();

        assert!(
            restore_remote_file_inner(&manager, "file-huge", "huge.md", &target_str, true)
                .await
                .is_err()
        );
        assert!(!target_dir.join("huge.md").exists());

        let batch = restore_all_remote_files_inner(&manager, &target_str, true)
            .await
            .unwrap();
        assert_eq!(batch.restored_count, 1);
        assert_eq!(batch.skipped_count, 2);
        assert!(batch.failed_files.is_empty());
        assert_eq!(fs::read_to_string(target_dir.join("ok.md")).unwrap(), "# OK");
        assert!(!dir.path().join("escape.md").exists());
        assert!(!target_dir.join("huge.md").exists());

        server_handle.abort();
    }
}
