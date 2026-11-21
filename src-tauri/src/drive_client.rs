use crate::history_store::HistoryStore;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::path::Path;
use std::time::Duration;

pub const MAX_BACKUP_BYTES: usize = 10 * 1024 * 1024;
pub const DRIVE_ROOT_FOLDER_NAME: &str = "OpenMD";
pub const DRIVE_FOLDER_MIME: &str = "application/vnd.google-apps.folder";
pub const MARKDOWN_MIME: &str = "text/markdown; charset=utf-8";
pub const MAX_LIST_ITEMS: usize = 5000;

const DRIVE_API_TIMEOUT: Duration = Duration::from_secs(30);
const DRIVE_TRANSFER_TIMEOUT: Duration = Duration::from_secs(60);

const PATH_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Configurable Google Drive REST v3 endpoints (overridable for mock HTTP tests).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveApiEndpoints {
    pub api_base_url: String,
    pub upload_base_url: String,
}

impl Default for DriveApiEndpoints {
    fn default() -> Self {
        Self {
            api_base_url: "https://www.googleapis.com/drive/v3".to_string(),
            upload_base_url: "https://upload.googleapis.com/upload/drive/v3".to_string(),
        }
    }
}

/// Remote Markdown/MDX file metadata returned by [`DriveClient::list_files_under_root`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveRemoteFile {
    pub file_id: String,
    pub name: String,
    pub remote_path: String,
    pub size_bytes: u64,
    pub modified_time_ms: Option<i64>,
}

/// Metadata returned after creating, updating, or moving a file on Google Drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFileMeta {
    pub id: String,
    pub parent_id: String,
    pub modified_time_ms: Option<i64>,
}

/// Structured Google Drive REST v3 API error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveApiError {
    Unauthorized(String),
    NotFound(String),
    RateLimited(String),
    Network(String),
    TooLarge(usize),
    Api { status: u16, message: String },
}

impl fmt::Display for DriveApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized(msg) => write!(f, "Drive API unauthorized: {msg}"),
            Self::NotFound(msg) => write!(f, "Drive API not found: {msg}"),
            Self::RateLimited(msg) => write!(f, "Drive API rate limited: {msg}"),
            Self::Network(msg) => write!(f, "Drive network error: {msg}"),
            Self::TooLarge(size) => write!(f, "File exceeds {MAX_BACKUP_BYTES} byte limit ({size})"),
            Self::Api { status, message } => write!(f, "Drive API error ({status}): {message}"),
        }
    }
}

impl std::error::Error for DriveApiError {}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFileListResponse {
    #[serde(default)]
    files: Option<Vec<DriveApiFileItem>>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveApiFileItem {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_u64")]
    size: Option<u64>,
    #[serde(default)]
    modified_time: Option<String>,
    #[serde(default)]
    parents: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFileStatusResponse {
    id: String,
    #[serde(default)]
    trashed: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DriveErrorEnvelope {
    error: Option<DriveErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct DriveErrorDetail {
    message: Option<String>,
}

/// HTTP client for Google Drive REST v3 folder caching, uploads, listing, and downloads.
#[derive(Debug, Clone)]
pub struct DriveClient {
    http: reqwest::Client,
    endpoints: DriveApiEndpoints,
}

impl DriveClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_endpoints(http, DriveApiEndpoints::default())
    }

    pub fn with_endpoints(http: reqwest::Client, endpoints: DriveApiEndpoints) -> Self {
        Self { http, endpoints }
    }

    /// Escapes backslashes and single quotes for Google Drive `q` query string literals.
    pub fn escape_drive_query_literal(value: &str) -> String {
        let mut out = String::with_capacity(value.len());
        for ch in value.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                _ => out.push(ch),
            }
        }
        out
    }

    /// Parses an RFC 3339 timestamp string into UTC Unix milliseconds without external date crates.
    pub fn parse_rfc3339_to_unix_ms(value: &str) -> Option<i64> {
        let s = value.trim();
        if !s.is_ascii() || s.len() < 20 {
            return None;
        }

        let (date_str, time_and_tz) = s.split_once(['T', 't'])?;
        let (year, month, day) = parse_iso_date(date_str)?;

        let (hms_frac, offset_secs) = if let Some(rest) = time_and_tz.strip_suffix(['Z', 'z']) {
            (rest, 0i64)
        } else {
            let pos = time_and_tz.rfind(['+', '-'])?;
            let (hms_part, tz_part) = time_and_tz.split_at(pos);
            (hms_part, parse_tz_offset_secs(tz_part)?)
        };

        let (hms_str, frac_ms) = match hms_frac.split_once('.') {
            Some((hms, frac)) => (hms, parse_fractional_ms(frac)?),
            None => (hms_frac, 0i64),
        };

        let (hour, minute, second) = parse_hms(hms_str)?;
        let days = days_from_civil(year, month as i64, day as i64);
        let day_secs = hour * 3600 + minute * 60 + second;
        let total_secs = days
            .checked_mul(86_400)?
            .checked_add(day_secs)?
            .checked_sub(offset_secs)?;
        total_secs.checked_mul(1000)?.checked_add(frac_ms)
    }

    /// Resolves or creates the top-level `OpenMD` folder in Drive and caches its ID locally.
    pub async fn ensure_root_folder(
        &self,
        access_token: &str,
        store: &HistoryStore,
        now_ms: i64,
    ) -> Result<String, DriveApiError> {
        if let Some(cached) = store
            .get_cached_folder_id("")
            .map_err(local_store_error)?
            .filter(|id| !id.trim().is_empty())
        {
            return Ok(cached);
        }

        let root_id = match self.find_root_folder_id(access_token).await? {
            Some(id) => id,
            None => {
                self.create_folder(access_token, DRIVE_ROOT_FOLDER_NAME, "root")
                    .await?
            }
        };

        store
            .upsert_cached_folder("", &root_id, Some("root"), now_ms)
            .map_err(local_store_error)?;
        store
            .set_drive_setting_str("drive_root_folder_id", &root_id)
            .map_err(local_store_error)?;
        Ok(root_id)
    }

    /// Resolves or creates a chain of folders beneath `OpenMD`, healing stale 404 cache entries.
    pub async fn ensure_folder_chain(
        &self,
        access_token: &str,
        store: &HistoryStore,
        folder_segments: &[&str],
        now_ms: i64,
    ) -> Result<String, DriveApiError> {
        let segments: Vec<&str> = folder_segments
            .iter()
            .copied()
            .filter(|s| !s.is_empty())
            .collect();

        if segments.is_empty() {
            return self.ensure_root_folder(access_token, store, now_ms).await;
        }

        // Bounded retries allow healing bottom-up when multiple cached ancestor levels vanished.
        let max_attempts = segments.len() + 2;
        for _ in 0..max_attempts {
            let root_from_cache = store
                .get_cached_folder_id("")
                .map_err(local_store_error)?
                .is_some_and(|id| !id.trim().is_empty());

            let mut current_id = self.ensure_root_folder(access_token, store, now_ms).await?;
            let mut current_path = String::new();
            let mut current_from_cache = root_from_cache;
            let mut healed_and_retry = false;

            for seg in &segments {
                let next_path = if current_path.is_empty() {
                    (*seg).to_string()
                } else {
                    format!("{current_path}/{seg}")
                };

                if let Some(cached) = store
                    .get_cached_folder_id(&next_path)
                    .map_err(local_store_error)?
                    .filter(|id| !id.trim().is_empty())
                {
                    current_id = cached;
                    current_path = next_path;
                    current_from_cache = true;
                    continue;
                }

                match self
                    .find_or_create_child_folder(access_token, seg, &current_id)
                    .await
                {
                    Ok(child_id) => {
                        store
                            .upsert_cached_folder(&next_path, &child_id, Some(&current_id), now_ms)
                            .map_err(local_store_error)?;
                        current_id = child_id;
                        current_path = next_path;
                        current_from_cache = false;
                    }
                    Err(DriveApiError::NotFound(_)) if current_from_cache => {
                        store
                            .invalidate_folder_prefix(&current_path)
                            .map_err(local_store_error)?;
                        if current_path.is_empty() {
                            store
                                .set_drive_setting_str("drive_root_folder_id", "")
                                .map_err(local_store_error)?;
                        }
                        healed_and_retry = true;
                        break;
                    }
                    Err(err) => return Err(err),
                }
            }

            if !healed_and_retry {
                return Ok(current_id);
            }
        }

        Err(DriveApiError::NotFound(
            "Drive folder hierarchy could not be resolved after cache invalidation".to_string(),
        ))
    }

    /// Uploads a new file, updates an existing file by ID or name lookup, or moves/renames it first.
    #[allow(clippy::too_many_arguments)]
    pub async fn upload_or_update_file(
        &self,
        access_token: &str,
        existing_file_id: Option<&str>,
        existing_parent_id: Option<&str>,
        target_parent_id: &str,
        file_name: &str,
        content: &str,
        rename_or_move: bool,
    ) -> Result<UploadedFileMeta, DriveApiError> {
        if content.len() > MAX_BACKUP_BYTES {
            return Err(DriveApiError::TooLarge(content.len()));
        }

        if let Some(file_id) = existing_file_id.filter(|id| !id.trim().is_empty()) {
            let effective_parent = if rename_or_move {
                let moved_parent = self
                    .patch_file_metadata(
                        access_token,
                        file_id,
                        existing_parent_id,
                        target_parent_id,
                        file_name,
                    )
                    .await?;
                if moved_parent.is_empty() {
                    target_parent_id.to_string()
                } else {
                    moved_parent
                }
            } else {
                target_parent_id.to_string()
            };

            return self
                .patch_file_media(access_token, file_id, &effective_parent, content)
                .await;
        }

        if let Some(found_id) = self
            .find_existing_file_in_parent(access_token, file_name, target_parent_id)
            .await?
        {
            return self
                .patch_file_media(access_token, &found_id, target_parent_id, content)
                .await;
        }

        self.create_file_multipart(access_token, target_parent_id, file_name, content)
            .await
    }

    /// Lists all Markdown/MDX files beneath `OpenMD/` using paginated BFS, newest first.
    pub async fn list_files_under_root(
        &self,
        access_token: &str,
        root_folder_id_hint: Option<&str>,
    ) -> Result<Vec<DriveRemoteFile>, DriveApiError> {
        let hinted = root_folder_id_hint
            .map(str::trim)
            .filter(|s| !s.is_empty());

        let resolved_root_id = match hinted {
            Some(hint) => match self.verify_active_folder_id(access_token, hint).await? {
                Some(active_id) => Some(active_id),
                None => self.find_root_folder_id(access_token).await?,
            },
            None => self.find_root_folder_id(access_token).await?,
        };

        let Some(root_id) = resolved_root_id else {
            return Ok(Vec::new());
        };

        self.bfs_list_from_root(access_token, &root_id).await
    }

    /// Downloads a Drive file's content as UTF-8 while enforcing [`MAX_BACKUP_BYTES`].
    pub async fn download_file_content(
        &self,
        access_token: &str,
        file_id: &str,
    ) -> Result<String, DriveApiError> {
        let mut resp = self
            .http
            .get(self.api_file_url(file_id)?)
            .timeout(DRIVE_TRANSFER_TIMEOUT)
            .bearer_auth(access_token)
            .query(&[("alt", "media")])
            .send()
            .await
            .map_err(|e| DriveApiError::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let err_body = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status, &err_body));
        }

        if let Some(len) = resp.content_length()
            && len > MAX_BACKUP_BYTES as u64
        {
            let reported = usize::try_from(len).unwrap_or(usize::MAX);
            return Err(DriveApiError::TooLarge(reported));
        }

        let initial_cap = resp
            .content_length()
            .and_then(|len| usize::try_from(len).ok())
            .map(|len| len.min(MAX_BACKUP_BYTES))
            .unwrap_or(0);
        let mut bytes = Vec::with_capacity(initial_cap);

        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| DriveApiError::Network(e.to_string()))?
        {
            let next_len = bytes.len().saturating_add(chunk.len());
            if next_len > MAX_BACKUP_BYTES {
                return Err(DriveApiError::TooLarge(next_len));
            }
            bytes.extend_from_slice(&chunk);
        }

        String::from_utf8(bytes).map_err(|e| DriveApiError::Api {
            status: status.as_u16(),
            message: format!("Downloaded file is not valid UTF-8: {e}"),
        })
    }

    fn api_files_url(&self) -> String {
        format!("{}/files", self.endpoints.api_base_url.trim_end_matches('/'))
    }

    fn api_file_url(&self, file_id: &str) -> Result<String, DriveApiError> {
        build_file_url(&self.endpoints.api_base_url, file_id)
    }

    fn upload_files_url(&self) -> String {
        format!(
            "{}/files",
            self.endpoints.upload_base_url.trim_end_matches('/')
        )
    }

    fn upload_file_url(&self, file_id: &str) -> Result<String, DriveApiError> {
        build_file_url(&self.endpoints.upload_base_url, file_id)
    }

    async fn verify_active_folder_id(
        &self,
        access_token: &str,
        folder_id: &str,
    ) -> Result<Option<String>, DriveApiError> {
        let req = self
            .http
            .get(self.api_file_url(folder_id)?)
            .timeout(DRIVE_API_TIMEOUT)
            .bearer_auth(access_token)
            .query(&[("fields", "id,trashed")]);

        match send_and_parse_json::<DriveFileStatusResponse>(req).await {
            Ok(meta) if meta.trashed == Some(true) || meta.id.trim().is_empty() => Ok(None),
            Ok(meta) => Ok(Some(meta.id)),
            Err(DriveApiError::NotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn find_root_folder_id(&self, access_token: &str) -> Result<Option<String>, DriveApiError> {
        let q = format!(
            "name = '{}' and mimeType = '{}' and 'root' in parents and trashed = false",
            Self::escape_drive_query_literal(DRIVE_ROOT_FOLDER_NAME),
            DRIVE_FOLDER_MIME
        );
        self.find_single_item_id(access_token, &q).await
    }

    async fn find_or_create_child_folder(
        &self,
        access_token: &str,
        segment: &str,
        parent_id: &str,
    ) -> Result<String, DriveApiError> {
        let q = format!(
            "name = '{}' and mimeType = '{}' and '{}' in parents and trashed = false",
            Self::escape_drive_query_literal(segment),
            DRIVE_FOLDER_MIME,
            Self::escape_drive_query_literal(parent_id)
        );
        if let Some(existing_id) = self.find_single_item_id(access_token, &q).await? {
            return Ok(existing_id);
        }
        self.create_folder(access_token, segment, parent_id).await
    }

    async fn find_existing_file_in_parent(
        &self,
        access_token: &str,
        file_name: &str,
        parent_id: &str,
    ) -> Result<Option<String>, DriveApiError> {
        let q = format!(
            "name = '{}' and '{}' in parents and mimeType != '{}' and trashed = false",
            Self::escape_drive_query_literal(file_name),
            Self::escape_drive_query_literal(parent_id),
            DRIVE_FOLDER_MIME
        );
        self.find_single_item_id(access_token, &q).await
    }

    async fn find_single_item_id(
        &self,
        access_token: &str,
        q: &str,
    ) -> Result<Option<String>, DriveApiError> {
        let req = self
            .http
            .get(self.api_files_url())
            .timeout(DRIVE_API_TIMEOUT)
            .bearer_auth(access_token)
            .query(&[
                ("q", q),
                ("orderBy", "modifiedTime desc"),
                ("pageSize", "1"),
                ("fields", "files(id,name)"),
            ]);
        let list: DriveFileListResponse = send_and_parse_json(req).await?;
        Ok(list
            .files
            .unwrap_or_default()
            .into_iter()
            .next()
            .map(|item| item.id))
    }

    async fn create_folder(
        &self,
        access_token: &str,
        name: &str,
        parent_id: &str,
    ) -> Result<String, DriveApiError> {
        let req = self
            .http
            .post(self.api_files_url())
            .timeout(DRIVE_API_TIMEOUT)
            .bearer_auth(access_token)
            .query(&[("fields", "id,name")])
            .json(&serde_json::json!({
                "name": name,
                "mimeType": DRIVE_FOLDER_MIME,
                "parents": [parent_id],
            }));
        let created: DriveApiFileItem = send_and_parse_json(req).await?;
        Ok(created.id)
    }

    async fn patch_file_metadata(
        &self,
        access_token: &str,
        file_id: &str,
        existing_parent_id: Option<&str>,
        target_parent_id: &str,
        file_name: &str,
    ) -> Result<String, DriveApiError> {
        let mut query_params: Vec<(&str, &str)> = Vec::new();
        if let Some(old_parent) =
            existing_parent_id.filter(|p| !p.is_empty() && *p != target_parent_id)
        {
            if !target_parent_id.is_empty() {
                query_params.push(("addParents", target_parent_id));
            }
            query_params.push(("removeParents", old_parent));
        } else if existing_parent_id.is_none() && !target_parent_id.is_empty() {
            query_params.push(("addParents", target_parent_id));
        }
        query_params.push(("fields", "id,name,parents,modifiedTime"));

        let req = self
            .http
            .patch(self.api_file_url(file_id)?)
            .timeout(DRIVE_API_TIMEOUT)
            .bearer_auth(access_token)
            .query(&query_params)
            .json(&serde_json::json!({
                "name": file_name,
            }));
        let updated: DriveApiFileItem = send_and_parse_json(req).await?;
        Ok(updated
            .parents
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_else(|| target_parent_id.to_string()))
    }

    async fn patch_file_media(
        &self,
        access_token: &str,
        file_id: &str,
        fallback_parent_id: &str,
        content: &str,
    ) -> Result<UploadedFileMeta, DriveApiError> {
        let req = self
            .http
            .patch(self.upload_file_url(file_id)?)
            .timeout(DRIVE_TRANSFER_TIMEOUT)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, MARKDOWN_MIME)
            .query(&[
                ("uploadType", "media"),
                ("fields", "id,name,parents,modifiedTime"),
            ])
            .body(content.as_bytes().to_vec());
        let item: DriveApiFileItem = send_and_parse_json(req).await?;
        Ok(uploaded_meta_from_item(item, fallback_parent_id))
    }

    async fn create_file_multipart(
        &self,
        access_token: &str,
        target_parent_id: &str,
        file_name: &str,
        content: &str,
    ) -> Result<UploadedFileMeta, DriveApiError> {
        let boundary = generate_multipart_boundary(content);
        let metadata_json = serde_json::to_string(&serde_json::json!({
            "name": file_name,
            "mimeType": MARKDOWN_MIME,
            "parents": [target_parent_id],
        }))
        .map_err(|e| DriveApiError::Api {
            status: 0,
            message: format!("Failed to serialize Drive upload metadata: {e}"),
        })?;

        let mut body = Vec::with_capacity(metadata_json.len() + content.len() + 256);
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
        body.extend_from_slice(metadata_json.as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
        body.extend_from_slice(format!("Content-Type: {MARKDOWN_MIME}\r\n\r\n").as_bytes());
        body.extend_from_slice(content.as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let content_type = format!("multipart/related; boundary={boundary}");
        let req = self
            .http
            .post(self.upload_files_url())
            .timeout(DRIVE_TRANSFER_TIMEOUT)
            .bearer_auth(access_token)
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .query(&[
                ("uploadType", "multipart"),
                ("fields", "id,name,parents,modifiedTime"),
            ])
            .body(body);

        let item: DriveApiFileItem = send_and_parse_json(req).await?;
        Ok(uploaded_meta_from_item(item, target_parent_id))
    }

    async fn bfs_list_from_root(
        &self,
        access_token: &str,
        root_id: &str,
    ) -> Result<Vec<DriveRemoteFile>, DriveApiError> {
        let mut queue: VecDeque<(String, String)> = VecDeque::new();
        let mut visited_folders: HashSet<String> = HashSet::new();
        let mut results: Vec<DriveRemoteFile> = Vec::new();
        let mut seen_items = 0usize;

        visited_folders.insert(root_id.to_string());
        queue.push_back((root_id.to_string(), String::new()));

        while let Some((folder_id, folder_prefix)) = queue.pop_front() {
            if seen_items >= MAX_LIST_ITEMS {
                break;
            }

            let q = format!(
                "'{}' in parents and trashed = false",
                Self::escape_drive_query_literal(&folder_id)
            );
            let mut page_token: Option<String> = None;
            let mut seen_tokens: HashSet<String> = HashSet::new();

            loop {
                if seen_items >= MAX_LIST_ITEMS {
                    break;
                }

                let mut query_params: Vec<(&str, &str)> = vec![
                    ("q", &q),
                    ("pageSize", "1000"),
                    (
                        "fields",
                        "nextPageToken,files(id,name,mimeType,size,modifiedTime,parents)",
                    ),
                ];
                if let Some(ref token) = page_token {
                    query_params.push(("pageToken", token));
                }

                let req = self
                    .http
                    .get(self.api_files_url())
                    .timeout(DRIVE_API_TIMEOUT)
                    .bearer_auth(access_token)
                    .query(&query_params);

                let page: DriveFileListResponse = match send_and_parse_json(req).await {
                    Ok(p) => p,
                    Err(DriveApiError::NotFound(_)) if folder_id != root_id => break,
                    Err(err) => return Err(err),
                };

                for item in page.files.unwrap_or_default() {
                    if seen_items >= MAX_LIST_ITEMS {
                        break;
                    }
                    seen_items += 1;

                    if item.mime_type.as_deref() == Some(DRIVE_FOLDER_MIME) {
                        if !item.name.is_empty() && visited_folders.insert(item.id.clone()) {
                            let next_prefix = if folder_prefix.is_empty() {
                                item.name
                            } else {
                                format!("{folder_prefix}/{}", item.name)
                            };
                            queue.push_back((item.id, next_prefix));
                        }
                        continue;
                    }

                    if !is_supported_markdown_filename(&item.name) {
                        continue;
                    }

                    let remote_path = if folder_prefix.is_empty() {
                        item.name.clone()
                    } else {
                        format!("{folder_prefix}/{}", item.name)
                    };
                    results.push(DriveRemoteFile {
                        file_id: item.id,
                        name: item.name,
                        remote_path,
                        size_bytes: item.size.unwrap_or(0),
                        modified_time_ms: item
                            .modified_time
                            .as_deref()
                            .and_then(Self::parse_rfc3339_to_unix_ms),
                    });
                }

                let Some(next_token) = page
                    .next_page_token
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                else {
                    break;
                };
                if !seen_tokens.insert(next_token.clone()) {
                    break;
                }
                page_token = Some(next_token);
            }
        }

        results.sort_by(|a, b| {
            b.modified_time_ms
                .cmp(&a.modified_time_ms)
                .then_with(|| a.remote_path.cmp(&b.remote_path))
        });
        Ok(results)
    }
}

fn local_store_error(err: String) -> DriveApiError {
    DriveApiError::Api {
        status: 0,
        message: format!("Local Drive cache error: {err}"),
    }
}

fn encode_path_segment(seg: &str) -> String {
    utf8_percent_encode(seg, PATH_ENCODE_SET).to_string()
}

fn build_file_url(base_url: &str, file_id: &str) -> Result<String, DriveApiError> {
    let trimmed = file_id.trim();
    if trimmed.is_empty() {
        return Err(DriveApiError::Api {
            status: 400,
            message: "Drive file ID cannot be empty".to_string(),
        });
    }
    Ok(format!(
        "{}/files/{}",
        base_url.trim_end_matches('/'),
        encode_path_segment(trimmed)
    ))
}

fn uploaded_meta_from_item(item: DriveApiFileItem, fallback_parent_id: &str) -> UploadedFileMeta {
    UploadedFileMeta {
        id: item.id,
        parent_id: item
            .parents
            .unwrap_or_default()
            .into_iter()
            .next()
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| fallback_parent_id.to_string()),
        modified_time_ms: item
            .modified_time
            .as_deref()
            .and_then(DriveClient::parse_rfc3339_to_unix_ms),
    }
}

fn generate_multipart_boundary(content: &str) -> String {
    loop {
        let mut bytes = [0u8; 16];
        rand::fill(&mut bytes);
        let mut hex = String::with_capacity(32);
        for byte in bytes {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
        }
        let boundary = format!("openmd_boundary_{hex}");
        if !content.contains(&boundary) {
            return boundary;
        }
    }
}

fn is_supported_markdown_filename(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "md" | "mdx" | "markdown"))
}

fn deserialize_optional_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrU64 {
        String(String),
        Num(u64),
    }

    match Option::<StringOrU64>::deserialize(deserializer)? {
        None => Ok(None),
        Some(StringOrU64::Num(n)) => Ok(Some(n)),
        Some(StringOrU64::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                trimmed
                    .parse::<u64>()
                    .map(Some)
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

async fn send_and_parse_json<T: serde::de::DeserializeOwned>(
    req: reqwest::RequestBuilder,
) -> Result<T, DriveApiError> {
    let resp = req
        .send()
        .await
        .map_err(|e| DriveApiError::Network(e.to_string()))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| DriveApiError::Network(e.to_string()))?;

    if !status.is_success() {
        return Err(map_http_error(status, &body));
    }

    serde_json::from_str::<T>(&body).map_err(|e| DriveApiError::Api {
        status: status.as_u16(),
        message: format!("Invalid Drive API JSON response: {e}"),
    })
}

fn map_http_error(status: reqwest::StatusCode, body: &str) -> DriveApiError {
    let parsed_msg = serde_json::from_str::<DriveErrorEnvelope>(body)
        .ok()
        .and_then(|env| env.error)
        .and_then(|err| err.message)
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty());

    let message = parsed_msg.unwrap_or_else(|| {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            status
                .canonical_reason()
                .unwrap_or("Drive API error")
                .to_string()
        } else {
            trimmed.to_string()
        }
    });

    match status.as_u16() {
        401 => DriveApiError::Unauthorized(message),
        404 => DriveApiError::NotFound(message),
        429 => DriveApiError::RateLimited(message),
        code => DriveApiError::Api {
            status: code,
            message,
        },
    }
}

fn parse_iso_date(date_str: &str) -> Option<(i64, u32, u32)> {
    let bytes = date_str.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    let year = parse_digits(&date_str[0..4])? as i64;
    let month = parse_digits(&date_str[5..7])? as u32;
    let day = parse_digits(&date_str[8..10])? as u32;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some((year, month, day))
}

fn parse_hms(hms_str: &str) -> Option<(i64, i64, i64)> {
    let bytes = hms_str.as_bytes();
    if bytes.len() != 8 || bytes[2] != b':' || bytes[5] != b':' {
        return None;
    }
    let hour = parse_digits(&hms_str[0..2])? as i64;
    let minute = parse_digits(&hms_str[3..5])? as i64;
    let second = parse_digits(&hms_str[6..8])? as i64;
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) || !(0..=60).contains(&second) {
        return None;
    }
    Some((hour, minute, second.min(59)))
}

fn parse_fractional_ms(frac: &str) -> Option<i64> {
    if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ms = 0i64;
    let weights = [100i64, 10, 1];
    for (digit, weight) in frac.bytes().take(3).zip(weights) {
        ms += i64::from(digit - b'0') * weight;
    }
    Some(ms)
}

fn parse_tz_offset_secs(tz: &str) -> Option<i64> {
    let bytes = tz.as_bytes();
    if bytes.len() != 6 || bytes[3] != b':' {
        return None;
    }
    let sign = match bytes[0] {
        b'+' => 1i64,
        b'-' => -1i64,
        _ => return None,
    };
    let hh = parse_digits(&tz[1..3])? as i64;
    let mm = parse_digits(&tz[4..6])? as i64;
    if !(0..=23).contains(&hh) || !(0..=59).contains(&mm) {
        return None;
    }
    Some(sign * (hh * 3600 + mm * 60))
}

fn parse_digits(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<u64>().ok()
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Howard Hinnant's proleptic Gregorian civil-date to Unix epoch day converter.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use percent_encoding::percent_decode_str;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
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
        override_size: Option<u64>,
        chunked_without_length: bool,
    }

    #[derive(Debug, Clone)]
    struct RecordedRequest {
        method: String,
        path: String,
        query: HashMap<String, String>,
        headers: HashMap<String, String>,
        body: Vec<u8>,
    }

    #[derive(Default)]
    struct MockDriveState {
        items: HashMap<String, MockItem>,
        next_id: usize,
        requests: Vec<RecordedRequest>,
        page_size_override: Option<usize>,
    }

    impl MockDriveState {
        fn new() -> Self {
            Self {
                next_id: 1,
                ..Self::default()
            }
        }

        fn alloc_id(&mut self, prefix: &str) -> String {
            let id = format!("{prefix}-{}", self.next_id);
            self.next_id += 1;
            id
        }

        fn add_folder(&mut self, id: &str, name: &str, parent_id: &str, modified_time: &str) {
            self.items.insert(
                id.to_string(),
                MockItem {
                    id: id.to_string(),
                    name: name.to_string(),
                    mime_type: DRIVE_FOLDER_MIME.to_string(),
                    parents: vec![parent_id.to_string()],
                    modified_time: modified_time.to_string(),
                    content: Vec::new(),
                    trashed: false,
                    override_size: None,
                    chunked_without_length: false,
                },
            );
        }

        fn add_file(
            &mut self,
            id: &str,
            name: &str,
            parent_id: &str,
            mime_type: &str,
            modified_time: &str,
            content: Vec<u8>,
        ) {
            self.items.insert(
                id.to_string(),
                MockItem {
                    id: id.to_string(),
                    name: name.to_string(),
                    mime_type: mime_type.to_string(),
                    parents: vec![parent_id.to_string()],
                    modified_time: modified_time.to_string(),
                    content,
                    trashed: false,
                    override_size: None,
                    chunked_without_length: false,
                },
            );
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

    async fn spawn_mock_drive_server(
        state: Arc<Mutex<MockDriveState>>,
    ) -> (DriveApiEndpoints, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");
        let endpoints = DriveApiEndpoints {
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
                    let (status, content_type, resp_bytes, chunked) =
                        handle_mock_request(&state, req);

                    let status_text = match status {
                        200 => "OK",
                        400 => "Bad Request",
                        401 => "Unauthorized",
                        404 => "Not Found",
                        429 => "Too Many Requests",
                        _ => "Internal Server Error",
                    };
                    if chunked {
                        let headers = format!(
                            "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(headers.as_bytes()).await;
                        for chunk in resp_bytes.chunks(1024 * 1024) {
                            let chunk_hdr = format!("{:x}\r\n", chunk.len());
                            if stream.write_all(chunk_hdr.as_bytes()).await.is_err() {
                                break;
                            }
                            if stream.write_all(chunk).await.is_err() {
                                break;
                            }
                            if stream.write_all(b"\r\n").await.is_err() {
                                break;
                            }
                        }
                        let _ = stream.write_all(b"0\r\n\r\n").await;
                        let _ = stream.flush().await;
                    } else {
                        let headers = format!(
                            "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            resp_bytes.len()
                        );
                        let _ = stream.write_all(headers.as_bytes()).await;
                        let _ = stream.write_all(&resp_bytes).await;
                        let _ = stream.flush().await;
                    }
                });
            }
        });

        (endpoints, handle)
    }

    fn handle_mock_request(
        state: &Arc<Mutex<MockDriveState>>,
        req: RecordedRequest,
    ) -> (u16, &'static str, Vec<u8>, bool) {
        let mut guard = state.lock().unwrap();
        guard.requests.push(req.clone());

        if req
            .headers
            .get("authorization")
            .is_some_and(|v| v == "Bearer bad-token")
            || !req
                .headers
                .get("authorization")
                .is_some_and(|v| v.starts_with("Bearer "))
        {
            return (
                401,
                "application/json",
                br#"{"error":{"message":"Unauthorized"}}"#.to_vec(),
                false,
            );
        }
        if req
            .headers
            .get("authorization")
            .is_some_and(|v| v == "Bearer rate-limited")
        {
            return (
                429,
                "application/json",
                br#"{"error":{"message":"Rate limit exceeded"}}"#.to_vec(),
                false,
            );
        }

        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/drive/v3/files") => {
                let q = req.query.get("q").cloned().unwrap_or_default();
                let parent = extract_in_parents(&q);

                let name_filter = extract_quoted_field(&q, "name = ");
                let mime_eq = extract_quoted_field(&q, "mimeType = ");
                let mime_neq = extract_quoted_field(&q, "mimeType != ");

                let mut matched: Vec<MockItem> = guard
                    .items
                    .values()
                    .filter(|item| !item.trashed)
                    .filter(|item| {
                        parent
                            .as_ref()
                            .is_none_or(|p| item.parents.iter().any(|ip| ip == p))
                    })
                    .filter(|item| name_filter.as_ref().is_none_or(|n| &item.name == n))
                    .filter(|item| mime_eq.as_ref().is_none_or(|m| &item.mime_type == m))
                    .filter(|item| mime_neq.as_ref().is_none_or(|m| &item.mime_type != m))
                    .cloned()
                    .collect();
                matched.sort_by(|a, b| a.id.cmp(&b.id));

                let offset = req
                    .query
                    .get("pageToken")
                    .and_then(|t| t.strip_prefix("p"))
                    .and_then(|n| n.parse::<usize>().ok())
                    .map(|p| p.saturating_sub(1))
                    .unwrap_or(0);
                let page_limit = guard.page_size_override.unwrap_or(1000);
                let slice_start = offset * page_limit;
                let page_items: Vec<MockItem> = matched
                    .iter()
                    .skip(slice_start)
                    .take(page_limit)
                    .cloned()
                    .collect();
                let next_token = if slice_start + page_items.len() < matched.len() {
                    Some(format!("p{}", offset + 2))
                } else {
                    None
                };

                let json_files: Vec<serde_json::Value> = page_items
                    .iter()
                    .map(|it| {
                        let size = it.override_size.unwrap_or(it.content.len() as u64);
                        serde_json::json!({
                            "id": it.id,
                            "name": it.name,
                            "mimeType": it.mime_type,
                            "size": size.to_string(),
                            "modifiedTime": it.modified_time,
                            "parents": it.parents,
                        })
                    })
                    .collect();
                let resp = serde_json::json!({
                    "files": json_files,
                    "nextPageToken": next_token,
                });
                (
                    200,
                    "application/json",
                    serde_json::to_vec(&resp).unwrap(),
                    false,
                )
            }
            ("POST", "/drive/v3/files") => {
                let body: serde_json::Value = match serde_json::from_slice(&req.body) {
                    Ok(v) => v,
                    Err(_) => {
                        return (
                            400,
                            "application/json",
                            br#"{"error":{"message":"Bad JSON"}}"#.to_vec(),
                            false,
                        );
                    }
                };
                let name = body
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mime_type = body
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or(DRIVE_FOLDER_MIME)
                    .to_string();
                let parents: Vec<String> = body
                    .get("parents")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_else(|| vec!["root".to_string()]);

                for p in &parents {
                    if p != "root" && !guard.items.get(p).is_some_and(|it| !it.trashed) {
                        return (
                            404,
                            "application/json",
                            br#"{"error":{"message":"Parent folder not found"}}"#.to_vec(),
                            false,
                        );
                    }
                }

                let id = guard.alloc_id("folder");
                let modified_time = "2026-11-21T15:00:00.000Z".to_string();
                guard.items.insert(
                    id.clone(),
                    MockItem {
                        id: id.clone(),
                        name: name.clone(),
                        mime_type,
                        parents: parents.clone(),
                        modified_time: modified_time.clone(),
                        content: Vec::new(),
                        trashed: false,
                        override_size: None,
                        chunked_without_length: false,
                    },
                );
                let resp = serde_json::json!({
                    "id": id,
                    "name": name,
                    "parents": parents,
                    "modifiedTime": modified_time,
                });
                (
                    200,
                    "application/json",
                    serde_json::to_vec(&resp).unwrap(),
                    false,
                )
            }
            ("POST", "/upload/drive/v3/files") => {
                let ctype = req
                    .headers
                    .get("content-type")
                    .map(String::as_str)
                    .unwrap_or("");
                let Some((meta_json, file_bytes)) = parse_multipart_related(ctype, &req.body)
                else {
                    return (
                        400,
                        "application/json",
                        br#"{"error":{"message":"Invalid multipart/related"}}"#.to_vec(),
                        false,
                    );
                };
                let meta: serde_json::Value = serde_json::from_str(&meta_json).unwrap();
                let name = meta
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mime_type = meta
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or(MARKDOWN_MIME)
                    .to_string();
                let parents: Vec<String> = meta
                    .get("parents")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                for p in &parents {
                    if p != "root" && !guard.items.get(p).is_some_and(|it| !it.trashed) {
                        return (
                            404,
                            "application/json",
                            br#"{"error":{"message":"Parent not found"}}"#.to_vec(),
                            false,
                        );
                    }
                }
                let id = guard.alloc_id("file");
                let modified_time = "2026-11-21T15:30:45.123Z".to_string();
                guard.items.insert(
                    id.clone(),
                    MockItem {
                        id: id.clone(),
                        name: name.clone(),
                        mime_type,
                        parents: parents.clone(),
                        modified_time: modified_time.clone(),
                        content: file_bytes,
                        trashed: false,
                        override_size: None,
                        chunked_without_length: false,
                    },
                );
                let resp = serde_json::json!({
                    "id": id,
                    "name": name,
                    "parents": parents,
                    "modifiedTime": modified_time,
                });
                (
                    200,
                    "application/json",
                    serde_json::to_vec(&resp).unwrap(),
                    false,
                )
            }
            ("PATCH", path) if path.starts_with("/drive/v3/files/") => {
                let id = path.trim_start_matches("/drive/v3/files/");
                let Some(existing) = guard.items.get(id).cloned() else {
                    return (
                        404,
                        "application/json",
                        br#"{"error":{"message":"File not found"}}"#.to_vec(),
                        false,
                    );
                };
                if existing.trashed {
                    return (
                        404,
                        "application/json",
                        br#"{"error":{"message":"File trashed"}}"#.to_vec(),
                        false,
                    );
                }
                let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
                if body.get("parents").is_some() {
                    return (
                        400,
                        "application/json",
                        br#"{"error":{"message":"parents field must not be in PATCH body"}}"#
                            .to_vec(),
                        false,
                    );
                }
                let mut parents = existing.parents.clone();
                if let Some(rem) = req.query.get("removeParents") {
                    for p in rem.split(',') {
                        parents.retain(|x| x != p);
                    }
                }
                if let Some(add) = req.query.get("addParents") {
                    for p in add.split(',') {
                        if !parents.iter().any(|x| x == p) {
                            parents.push(p.to_string());
                        }
                    }
                }
                let new_name = body
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&existing.name)
                    .to_string();
                let item = guard.items.get_mut(id).unwrap();
                item.name = new_name.clone();
                item.parents = parents.clone();
                let modified_time = item.modified_time.clone();
                let resp = serde_json::json!({
                    "id": id,
                    "name": new_name,
                    "parents": parents,
                    "modifiedTime": modified_time,
                });
                (
                    200,
                    "application/json",
                    serde_json::to_vec(&resp).unwrap(),
                    false,
                )
            }
            ("PATCH", path) if path.starts_with("/upload/drive/v3/files/") => {
                let id = path.trim_start_matches("/upload/drive/v3/files/");
                let Some(item) = guard.items.get_mut(id) else {
                    return (
                        404,
                        "application/json",
                        br#"{"error":{"message":"File not found"}}"#.to_vec(),
                        false,
                    );
                };
                if item.trashed {
                    return (
                        404,
                        "application/json",
                        br#"{"error":{"message":"File trashed"}}"#.to_vec(),
                        false,
                    );
                }
                item.content = req.body.clone();
                item.modified_time = "2026-11-21T16:00:00.000Z".to_string();
                let resp = serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "parents": item.parents,
                    "modifiedTime": item.modified_time,
                });
                (
                    200,
                    "application/json",
                    serde_json::to_vec(&resp).unwrap(),
                    false,
                )
            }
            ("GET", path) if path.starts_with("/drive/v3/files/") => {
                let id = path.trim_start_matches("/drive/v3/files/");
                let Some(item) = guard.items.get(id) else {
                    return (
                        404,
                        "application/json",
                        br#"{"error":{"message":"File not found"}}"#.to_vec(),
                        false,
                    );
                };
                if req.query.get("alt").map(String::as_str) == Some("media") {
                    if item.trashed {
                        return (
                            404,
                            "application/json",
                            br#"{"error":{"message":"File trashed"}}"#.to_vec(),
                            false,
                        );
                    }
                    return (
                        200,
                        "text/markdown; charset=utf-8",
                        item.content.clone(),
                        item.chunked_without_length,
                    );
                }
                let meta = serde_json::json!({
                    "id": item.id,
                    "name": item.name,
                    "mimeType": item.mime_type,
                    "trashed": item.trashed,
                    "parents": item.parents,
                });
                (
                    200,
                    "application/json",
                    serde_json::to_vec(&meta).unwrap(),
                    false,
                )
            }
            _ => (
                404,
                "application/json",
                br#"{"error":{"message":"Route not found"}}"#.to_vec(),
                false,
            ),
        }
    }

    #[test]
    fn test_escape_drive_query_literal_quotes_and_backslashes() {
        assert_eq!(
            DriveClient::escape_drive_query_literal(r"O'Reilly\Docs"),
            r"O\'Reilly\\Docs"
        );
        assert_eq!(
            DriveClient::escape_drive_query_literal("O'Reilly'nin Notları/Türkçe.md"),
            r"O\'Reilly\'nin Notları/Türkçe.md"
        );
    }

    #[test]
    fn test_parse_rfc3339_to_unix_ms() {
        assert_eq!(
            DriveClient::parse_rfc3339_to_unix_ms("1970-01-01T00:00:00Z"),
            Some(0)
        );
        assert_eq!(
            DriveClient::parse_rfc3339_to_unix_ms("2024-02-29T12:00:00Z"),
            Some(1_709_208_000_000)
        );
        assert_eq!(
            DriveClient::parse_rfc3339_to_unix_ms("2026-11-21T15:30:45.123Z"),
            Some(1_795_275_045_123)
        );
        assert_eq!(
            DriveClient::parse_rfc3339_to_unix_ms("2026-11-21T18:30:45.123+03:00"),
            Some(1_795_275_045_123)
        );
        assert_eq!(
            DriveClient::parse_rfc3339_to_unix_ms("2023-02-29T12:00:00Z"),
            None
        );
        assert_eq!(DriveClient::parse_rfc3339_to_unix_ms("not-a-date"), None);
    }

    #[tokio::test]
    async fn test_ensure_folder_chain_caches_and_heals_404() {
        let dir = tempdir().unwrap();
        let store = HistoryStore::open(dir.path().join("history.db")).unwrap();
        let state = Arc::new(Mutex::new(MockDriveState::new()));
        let (endpoints, server) = spawn_mock_drive_server(Arc::clone(&state)).await;
        let client = DriveClient::with_endpoints(reqwest::Client::new(), endpoints);

        let notes_id_1 = client
            .ensure_folder_chain("tok", &store, &["Documents", "Notes"], 1_000)
            .await
            .unwrap();
        assert!(!notes_id_1.is_empty());
        assert!(store.get_cached_folder_id("").unwrap().is_some());
        assert!(store.get_cached_folder_id("Documents").unwrap().is_some());
        assert_eq!(
            store.get_cached_folder_id("Documents/Notes").unwrap(),
            Some(notes_id_1.clone())
        );
        assert_eq!(
            store.get_drive_setting_str("drive_root_folder_id").unwrap(),
            store.get_cached_folder_id("").unwrap().unwrap()
        );

        let hits_after_first = state.lock().unwrap().requests.len();
        assert!(hits_after_first >= 3);

        // Second call must hit SQLite cache only (zero mock HTTP requests).
        let notes_id_2 = client
            .ensure_folder_chain("tok", &store, &["Documents", "Notes"], 2_000)
            .await
            .unwrap();
        assert_eq!(notes_id_2, notes_id_1);
        assert_eq!(state.lock().unwrap().requests.len(), hits_after_first);

        // Delete Notes on the Drive server and verify 404 self-healing via invalidate_folder_prefix.
        {
            let mut guard = state.lock().unwrap();
            guard.items.remove(&notes_id_1);
        }
        let child_id = client
            .ensure_folder_chain("tok", &store, &["Documents", "Notes", "Child"], 3_000)
            .await
            .unwrap();
        assert!(!child_id.is_empty());
        let healed_notes_id = store
            .get_cached_folder_id("Documents/Notes")
            .unwrap()
            .unwrap();
        assert_ne!(healed_notes_id, notes_id_1);

        // Also test root 404 healing clearing and re-populating drive_root_folder_id.
        let old_root_id = store.get_cached_folder_id("").unwrap().unwrap();
        {
            let mut guard = state.lock().unwrap();
            guard.items.clear();
        }
        let healed_other_id = client
            .ensure_folder_chain("tok", &store, &["Archive"], 4_000)
            .await
            .unwrap();
        assert!(!healed_other_id.is_empty());
        let new_root_id = store.get_cached_folder_id("").unwrap().unwrap();
        assert_ne!(new_root_id, old_root_id);
        assert_eq!(
            store.get_drive_setting_str("drive_root_folder_id").unwrap(),
            new_root_id
        );

        // Folder segment containing quotes, backslashes, and Turkish UTF-8 resolves & caches cleanly.
        let special_id = client
            .ensure_folder_chain("tok", &store, &[r"O'Reilly\Notları", "Türkçe"], 5_000)
            .await
            .unwrap();
        assert!(!special_id.is_empty());
        assert_eq!(
            store
                .get_cached_folder_id(r"O'Reilly\Notları/Türkçe")
                .unwrap(),
            Some(special_id)
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_upload_multipart_and_media_patch_with_turkish_and_quotes() {
        let state = Arc::new(Mutex::new(MockDriveState::new()));
        {
            let mut guard = state.lock().unwrap();
            guard.add_folder("parent-1", "Notlar", "root", "2026-11-21T10:00:00Z");
        }
        let (endpoints, server) = spawn_mock_drive_server(Arc::clone(&state)).await;
        let client = DriveClient::with_endpoints(reqwest::Client::new(), endpoints);

        let file_name = "O'Reilly'nin Notları.md";
        let content_v1 = "İçerik\n";

        let created = client
            .upload_or_update_file(
                "tok",
                None,
                None,
                "parent-1",
                file_name,
                content_v1,
                false,
            )
            .await
            .unwrap();
        assert_eq!(created.parent_id, "parent-1");
        assert!(created.modified_time_ms.is_some());

        {
            let guard = state.lock().unwrap();
            let list_req = guard
                .requests
                .iter()
                .find(|r| r.method == "GET" && r.path == "/drive/v3/files")
                .expect("expected pre-create existence search");
            let q = list_req.query.get("q").unwrap();
            assert!(
                q.contains(r"O\'Reilly\'nin Notları.md"),
                "query did not escape single quotes: {q}"
            );

            let multipart_req = guard
                .requests
                .iter()
                .find(|r| r.method == "POST" && r.path == "/upload/drive/v3/files")
                .expect("expected multipart POST");
            assert_eq!(
                multipart_req.query.get("uploadType").map(String::as_str),
                Some("multipart")
            );
            let ctype = multipart_req.headers.get("content-type").unwrap();
            assert!(ctype.starts_with("multipart/related; boundary="));
            let (meta_raw, body_bytes) =
                parse_multipart_related(ctype, &multipart_req.body).unwrap();
            let meta: serde_json::Value = serde_json::from_str(&meta_raw).unwrap();
            assert_eq!(meta.get("name").and_then(|v| v.as_str()), Some(file_name));
            assert_eq!(
                meta.get("mimeType").and_then(|v| v.as_str()),
                Some(MARKDOWN_MIME)
            );
            assert_eq!(String::from_utf8(body_bytes).unwrap(), content_v1);
        }

        // Second call with existing_file_id -> PATCH uploadType=media
        let content_v2 = "Güncel Türkçe İçerik\n";
        let updated = client
            .upload_or_update_file(
                "tok",
                Some(&created.id),
                Some("parent-1"),
                "parent-1",
                file_name,
                content_v2,
                false,
            )
            .await
            .unwrap();
        assert_eq!(updated.id, created.id);

        // Call with existing_file_id: None for same file name in parent -> finds existing & PATCHes instead of duplicate POST
        let content_v3 = "Üçüncü Sürüm\n";
        let deduped = client
            .upload_or_update_file(
                "tok",
                None,
                None,
                "parent-1",
                file_name,
                content_v3,
                false,
            )
            .await
            .unwrap();
        assert_eq!(deduped.id, created.id);

        let guard = state.lock().unwrap();
        let post_count = guard
            .requests
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/upload/drive/v3/files")
            .count();
        let patch_count = guard
            .requests
            .iter()
            .filter(|r| {
                r.method == "PATCH"
                    && r.path == format!("/upload/drive/v3/files/{}", created.id)
                    && r.query.get("uploadType").map(String::as_str) == Some("media")
            })
            .count();
        assert_eq!(post_count, 1);
        assert_eq!(patch_count, 2);
        assert_eq!(
            String::from_utf8(guard.items.get(&created.id).unwrap().content.clone()).unwrap(),
            content_v3
        );

        server.abort();
    }

    #[tokio::test]
    async fn test_move_or_rename_uses_add_remove_parents_query_params() {
        let state = Arc::new(Mutex::new(MockDriveState::new()));
        {
            let mut guard = state.lock().unwrap();
            guard.add_folder("p1", "Eski", "root", "2026-11-21T10:00:00Z");
            guard.add_folder("p2", "Yeni", "root", "2026-11-21T10:00:00Z");
            guard.add_file(
                "f1",
                "Eski Ad.md",
                "p1",
                MARKDOWN_MIME,
                "2026-11-21T10:00:00Z",
                b"old".to_vec(),
            );
        }
        let (endpoints, server) = spawn_mock_drive_server(Arc::clone(&state)).await;
        let client = DriveClient::with_endpoints(reqwest::Client::new(), endpoints);

        let meta = client
            .upload_or_update_file(
                "tok",
                Some("f1"),
                Some("p1"),
                "p2",
                "Yeni Ad.md",
                "moved content",
                true,
            )
            .await
            .unwrap();
        assert_eq!(meta.id, "f1");
        assert_eq!(meta.parent_id, "p2");

        let guard = state.lock().unwrap();
        let patch_meta_idx = guard
            .requests
            .iter()
            .position(|r| r.method == "PATCH" && r.path == "/drive/v3/files/f1")
            .expect("expected metadata PATCH /files/f1");
        let patch_media_idx = guard
            .requests
            .iter()
            .position(|r| {
                r.method == "PATCH"
                    && r.path == "/upload/drive/v3/files/f1"
                    && r.query.get("uploadType").map(String::as_str) == Some("media")
            })
            .expect("expected media PATCH after metadata PATCH");
        assert!(patch_meta_idx < patch_media_idx);

        let meta_req = &guard.requests[patch_meta_idx];
        assert_eq!(
            meta_req.query.get("addParents").map(String::as_str),
            Some("p2")
        );
        assert_eq!(
            meta_req.query.get("removeParents").map(String::as_str),
            Some("p1")
        );
        let body_json: serde_json::Value = serde_json::from_slice(&meta_req.body).unwrap();
        assert_eq!(
            body_json.get("name").and_then(|v| v.as_str()),
            Some("Yeni Ad.md")
        );
        assert!(body_json.get("parents").is_none());

        server.abort();
    }

    #[tokio::test]
    async fn test_list_files_under_root_paginated_bfs() {
        let state = Arc::new(Mutex::new(MockDriveState::new()));
        {
            let mut guard = state.lock().unwrap();
            guard.page_size_override = Some(1);
            guard.add_folder("root-openmd", "OpenMD", "root", "2026-11-21T10:00:00Z");
            guard.add_folder("fld-docs", "Documents", "root-openmd", "2026-11-21T10:05:00Z");
            guard.add_file(
                "file-b",
                "b.mdx",
                "root-openmd",
                MARKDOWN_MIME,
                "2026-11-21T12:00:00.000Z",
                b"b".to_vec(),
            );
            guard.add_file(
                "file-png",
                "logo.png",
                "root-openmd",
                "image/png",
                "2026-11-21T13:00:00.000Z",
                b"png".to_vec(),
            );
            guard.add_file(
                "file-a",
                "a.md",
                "fld-docs",
                MARKDOWN_MIME,
                "2026-11-21T14:00:00.000Z",
                b"aa".to_vec(),
            );
        }
        let (endpoints, server) = spawn_mock_drive_server(Arc::clone(&state)).await;
        let client = DriveClient::with_endpoints(reqwest::Client::new(), endpoints);

        let files = client.list_files_under_root("tok", None).await.unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_id, "file-a");
        assert_eq!(files[0].remote_path, "Documents/a.md");
        assert_eq!(files[0].size_bytes, 2);
        assert_eq!(files[1].file_id, "file-b");
        assert_eq!(files[1].remote_path, "b.mdx");
        assert_eq!(files[1].size_bytes, 1);

        let used_page_token = state
            .lock()
            .unwrap()
            .requests
            .iter()
            .any(|r| r.query.get("pageToken").map(String::as_str) == Some("p2"));
        assert!(used_page_token);

        // Active root hint passes files.get verification and skips searching root.
        let via_active_hint = client
            .list_files_under_root("tok", Some("root-openmd"))
            .await
            .unwrap();
        assert_eq!(via_active_hint, files);

        // Stale root hint returning 404 on files.get falls back to searching OpenMD root.
        let via_stale_hint = client
            .list_files_under_root("tok", Some("missing-root-hint"))
            .await
            .unwrap();
        assert_eq!(via_stale_hint, files);
        assert!(state.lock().unwrap().requests.iter().any(|r| {
            r.method == "GET"
                && r.path == "/drive/v3/files/missing-root-hint"
                && r.query.get("fields").map(String::as_str) == Some("id,trashed")
        }));

        // Trashed root hint (even with non-trashed descendants) falls back to active OpenMD root.
        {
            let mut guard = state.lock().unwrap();
            guard.add_folder("trashed-openmd", "OpenMD", "root", "2026-11-21T09:00:00Z");
            guard.items.get_mut("trashed-openmd").unwrap().trashed = true;
            guard.add_file(
                "file-in-trashed",
                "ghost.md",
                "trashed-openmd",
                MARKDOWN_MIME,
                "2026-11-21T15:00:00.000Z",
                b"ghost".to_vec(),
            );
        }
        let via_trashed_hint = client
            .list_files_under_root("tok", Some("trashed-openmd"))
            .await
            .unwrap();
        assert_eq!(via_trashed_hint, files);

        // When no active OpenMD root exists, stale/trashed hints return an empty list rather than failing.
        state.lock().unwrap().items.remove("root-openmd");
        state.lock().unwrap().items.remove("fld-docs");
        let empty_from_trashed = client
            .list_files_under_root("tok", Some("trashed-openmd"))
            .await
            .unwrap();
        assert!(empty_from_trashed.is_empty());

        state.lock().unwrap().items.clear();
        let empty = client.list_files_under_root("tok", None).await.unwrap();
        assert!(empty.is_empty());
        let empty_from_missing = client
            .list_files_under_root("tok", Some("missing-root-hint"))
            .await
            .unwrap();
        assert!(empty_from_missing.is_empty());

        server.abort();
    }

    #[tokio::test]
    async fn test_download_file_content_enforces_10mib_limit() {
        let state = Arc::new(Mutex::new(MockDriveState::new()));
        {
            let mut guard = state.lock().unwrap();
            guard.add_file(
                "f-ok",
                "ok.md",
                "root",
                MARKDOWN_MIME,
                "2026-11-21T10:00:00Z",
                "Merhaba Dünya\n".as_bytes().to_vec(),
            );
            guard.add_file(
                "f-big",
                "big.md",
                "root",
                MARKDOWN_MIME,
                "2026-11-21T10:00:00Z",
                vec![b'a'; MAX_BACKUP_BYTES + 1],
            );
            let chunked_big = MockItem {
                id: "f-big-chunked".to_string(),
                name: "chunked.md".to_string(),
                mime_type: MARKDOWN_MIME.to_string(),
                parents: vec!["root".to_string()],
                modified_time: "2026-11-21T10:00:00Z".to_string(),
                content: vec![b'b'; MAX_BACKUP_BYTES + 1],
                trashed: false,
                override_size: None,
                chunked_without_length: true,
            };
            guard.items.insert("f-big-chunked".to_string(), chunked_big);
        }
        let (endpoints, server) = spawn_mock_drive_server(Arc::clone(&state)).await;
        let client = DriveClient::with_endpoints(reqwest::Client::new(), endpoints);

        let ok = client.download_file_content("tok", "f-ok").await.unwrap();
        assert_eq!(ok, "Merhaba Dünya\n");

        let err_len = client
            .download_file_content("tok", "f-big")
            .await
            .unwrap_err();
        assert!(matches!(err_len, DriveApiError::TooLarge(n) if n == MAX_BACKUP_BYTES + 1));

        let err_chunked = client
            .download_file_content("tok", "f-big-chunked")
            .await
            .unwrap_err();
        assert!(matches!(err_chunked, DriveApiError::TooLarge(n) if n > MAX_BACKUP_BYTES));

        let err_404 = client
            .download_file_content("tok", "does-not-exist")
            .await
            .unwrap_err();
        assert!(matches!(err_404, DriveApiError::NotFound(_)));

        let err_401 = client
            .download_file_content("bad-token", "f-ok")
            .await
            .unwrap_err();
        assert!(matches!(err_401, DriveApiError::Unauthorized(_)));

        let err_429 = client
            .download_file_content("rate-limited", "f-ok")
            .await
            .unwrap_err();
        assert!(matches!(err_429, DriveApiError::RateLimited(_)));

        let err_empty = client
            .download_file_content("tok", "   ")
            .await
            .unwrap_err();
        assert!(matches!(err_empty, DriveApiError::Api { status: 400, .. }));

        server.abort();
        let _ = server.await;

        let err_net = client
            .download_file_content("tok", "f-ok")
            .await
            .unwrap_err();
        assert!(matches!(err_net, DriveApiError::Network(_)));
    }
}
