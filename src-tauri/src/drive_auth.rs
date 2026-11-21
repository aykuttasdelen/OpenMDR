use crate::history_store::HistoryStore;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Mutex as AsyncMutex};

pub const KEYCHAIN_SERVICE: &str = "com.openmd.editor";
pub const KEYCHAIN_OAUTH_ACCOUNT: &str = "google-drive-oauth";
pub const DRIVE_OAUTH_SCOPES: &str = "https://www.googleapis.com/auth/drive.file openid email";

const OAUTH_LOOPBACK_TIMEOUT: Duration = Duration::from_secs(180);
const LOOPBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);
const OAUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(15);
const OAUTH_REVOKE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LOOPBACK_REQUEST_BYTES: usize = 16 * 1024;
const TOKEN_REFRESH_LEEWAY_MS: i64 = 60_000;

/// RFC 3986 unreserved characters are excluded from percent-encoding.
const QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Abstraction over secure credential persistence (macOS Keychain in prod, in-memory in tests).
pub trait TokenStore: Send + Sync {
    fn load_tokens(&self) -> Result<Option<StoredOAuthTokens>, String>;
    fn save_tokens(&self, tokens: &StoredOAuthTokens) -> Result<(), String>;
    fn delete_tokens(&self) -> Result<(), String>;
}

/// OS Keychain-backed [`TokenStore`] using service `"com.openmd.editor"`.
pub struct KeychainTokenStore;

impl KeychainTokenStore {
    fn entry(account: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, account).map_err(|e| e.to_string())
    }

    fn read_optional_password(account: &str) -> Result<Option<String>, String> {
        let entry = Self::entry(account)?;
        match entry.get_password() {
            Ok(secret) if !secret.is_empty() => Ok(Some(secret)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    fn delete_if_exists(account: &str) -> Result<(), String> {
        let entry = Self::entry(account)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

impl TokenStore for KeychainTokenStore {
    fn load_tokens(&self) -> Result<Option<StoredOAuthTokens>, String> {
        let Some(raw) = Self::read_optional_password(KEYCHAIN_OAUTH_ACCOUNT)? else {
            return Ok(None);
        };
        if raw.trim().is_empty() {
            return Ok(None);
        }
        serde_json::from_str::<StoredOAuthTokens>(&raw)
            .map(Some)
            .map_err(|e| format!("Failed to parse stored OAuth tokens: {e}"))
    }

    fn save_tokens(&self, tokens: &StoredOAuthTokens) -> Result<(), String> {
        let json = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        Self::entry(KEYCHAIN_OAUTH_ACCOUNT)?
            .set_password(&json)
            .map_err(|e| e.to_string())
    }

    fn delete_tokens(&self) -> Result<(), String> {
        Self::delete_if_exists(KEYCHAIN_OAUTH_ACCOUNT)
    }
}

/// Thread-safe in-memory [`TokenStore`] test double.
#[derive(Debug, Default)]
pub struct MemoryTokenStore {
    tokens: StdMutex<Option<StoredOAuthTokens>>,
}

impl MemoryTokenStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load_tokens(&self) -> Result<Option<StoredOAuthTokens>, String> {
        Ok(self.tokens.lock().map_err(|e| e.to_string())?.clone())
    }

    fn save_tokens(&self, tokens: &StoredOAuthTokens) -> Result<(), String> {
        *self.tokens.lock().map_err(|e| e.to_string())? = Some(tokens.clone());
        Ok(())
    }

    fn delete_tokens(&self) -> Result<(), String> {
        *self.tokens.lock().map_err(|e| e.to_string())? = None;
        Ok(())
    }
}

/// Configurable Google OAuth 2.0 endpoints (overridable for mock HTTP server tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthEndpoints {
    pub auth_url: String,
    pub token_url: String,
    pub revoke_url: String,
    pub userinfo_url: String,
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self {
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
            token_url: "https://oauth2.googleapis.com/token".to_string(),
            revoke_url: "https://oauth2.googleapis.com/revoke".to_string(),
            userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo".to_string(),
        }
    }
}

/// Resolved built-in OAuth client credentials from compile-time configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClientCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// OAuth token bundle persisted in the OS Keychain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StoredOAuthTokens {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at_ms: i64,
    pub email: String,
}

/// Structured OAuth lifecycle error returned by [`DriveAuthManager`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    NotConfigured,
    Cancelled,
    Timeout,
    Denied(String),
    StateMismatch,
    InvalidGrant(String),
    Network(String),
    Other(String),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(f, "OAUTH_CLIENT_NOT_CONFIGURED"),
            Self::Cancelled => write!(f, "AUTH_CANCELLED"),
            Self::Timeout => write!(f, "AUTH_TIMEOUT"),
            Self::Denied(reason) if reason.is_empty() => write!(f, "AUTH_DENIED"),
            Self::Denied(reason) => write!(f, "AUTH_DENIED: {reason}"),
            Self::StateMismatch => write!(f, "AUTH_STATE_MISMATCH"),
            Self::InvalidGrant(msg) => write!(f, "INVALID_GRANT: {msg}"),
            Self::Network(msg) => write!(f, "NETWORK_ERROR: {msg}"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for AuthError {}

struct ActiveAuthSession {
    id: u64,
    cancel_tx: watch::Sender<bool>,
}

/// Ensures an in-flight session slot is cleared only if this session still owns it.
struct SessionGuard<'a> {
    active_auth: &'a StdMutex<Option<ActiveAuthSession>>,
    session_id: u64,
}

impl Drop for SessionGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.active_auth.lock()
            && guard.as_ref().is_some_and(|s| s.id == self.session_id)
        {
            *guard = None;
        }
    }
}

#[derive(Deserialize)]
struct TokenEndpointResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

#[derive(Deserialize)]
struct OAuthErrorBody {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct UserInfoResponse {
    #[serde(default)]
    email: Option<String>,
}

struct LoopbackHttpRequest {
    method: String,
    path: String,
    query: String,
}

/// Manages OAuth client configuration, PKCE loopback sign-in, and serialized token refresh.
pub struct DriveAuthManager {
    store: Arc<HistoryStore>,
    token_store: Arc<dyn TokenStore>,
    http: reqwest::Client,
    endpoints: OAuthEndpoints,
    built_in_client_id: Option<String>,
    built_in_client_secret: Option<String>,
    refresh_lock: AsyncMutex<()>,
    active_auth: StdMutex<Option<ActiveAuthSession>>,
    next_session_id: AtomicU64,
}

impl DriveAuthManager {
    pub fn new(
        store: Arc<HistoryStore>,
        token_store: Arc<dyn TokenStore>,
        http: reqwest::Client,
        endpoints: OAuthEndpoints,
    ) -> Self {
        let built_in_client_id = option_env!("OPENMD_GOOGLE_CLIENT_ID")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let built_in_client_secret = option_env!("OPENMD_GOOGLE_CLIENT_SECRET")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Self {
            store,
            token_store,
            http,
            endpoints,
            built_in_client_id,
            built_in_client_secret,
            refresh_lock: AsyncMutex::new(()),
            active_auth: StdMutex::new(None),
            next_session_id: AtomicU64::new(1),
        }
    }

    pub fn with_built_in_credentials(
        mut self,
        client_id: Option<String>,
        client_secret: Option<String>,
    ) -> Self {
        self.built_in_client_id = client_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self.built_in_client_secret = client_secret
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        self
    }

    pub fn is_configured(&self) -> bool {
        self.built_in_client_id
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    }

    pub fn resolve_credentials(&self) -> Result<OAuthClientCredentials, AuthError> {
        if let Some(built_in_id) = self.built_in_client_id.as_deref().filter(|s| !s.is_empty()) {
            return Ok(OAuthClientCredentials {
                client_id: built_in_id.to_string(),
                client_secret: self.built_in_client_secret.clone(),
            });
        }

        Err(AuthError::NotConfigured)
    }

    pub fn has_stored_tokens(&self) -> bool {
        matches!(
            self.token_store.load_tokens(),
            Ok(Some(tokens))
                if !tokens.refresh_token.trim().is_empty()
                    || !tokens.access_token.trim().is_empty()
        )
    }

    pub fn is_auth_in_progress(&self) -> bool {
        self.active_auth
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    pub async fn bind_loopback_session<F>(
        &self,
        open_url: F,
    ) -> Result<(StoredOAuthTokens, String), AuthError>
    where
        F: FnOnce(&str) -> Result<(), String>,
    {
        let creds = self.resolve_credentials()?;
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| AuthError::Other(format!("Failed to bind OAuth loopback listener: {e}")))?;
        let port = listener
            .local_addr()
            .map_err(|e| AuthError::Other(format!("Failed to read loopback port: {e}")))?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");

        let (code_verifier, code_challenge) = generate_pkce_pair();
        let expected_state = generate_state();
        let auth_url = build_authorization_url(
            &self.endpoints.auth_url,
            &creds.client_id,
            &redirect_uri,
            &expected_state,
            &code_challenge,
        );

        let session_id = self.next_session_id.fetch_add(1, Ordering::SeqCst);
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        {
            let mut guard = self
                .active_auth
                .lock()
                .map_err(|e| AuthError::Other(e.to_string()))?;
            if let Some(prev) = guard.take() {
                let _ = prev.cancel_tx.send(true);
            }
            *guard = Some(ActiveAuthSession {
                id: session_id,
                cancel_tx,
            });
        }
        let _session_guard = SessionGuard {
            active_auth: &self.active_auth,
            session_id,
        };

        open_url(&auth_url).map_err(AuthError::Other)?;

        let deadline = tokio::time::sleep(OAUTH_LOOPBACK_TIMEOUT);
        tokio::pin!(deadline);

        let (mut stream, req) = loop {
            tokio::select! {
                biased;
                _ = wait_for_cancel(&mut cancel_rx) => {
                    return Err(AuthError::Cancelled);
                }
                _ = &mut deadline => {
                    return Err(AuthError::Timeout);
                }
                accept_res = listener.accept() => {
                    let (mut stream, _) = accept_res
                        .map_err(|e| AuthError::Other(format!("Loopback accept failed: {e}")))?;
                    let Some(req) = read_loopback_http_request(&mut stream).await else {
                        continue;
                    };
                    if req.method != "GET" || req.path != "/oauth/callback" {
                        let _ = write_html_response(
                            &mut stream,
                            404,
                            "Not Found",
                            "Not Found",
                            "This endpoint only accepts OAuth callbacks.",
                        )
                        .await;
                        continue;
                    }
                    break (stream, req);
                }
            }
        };

        let params = parse_query_params(&req.query);
        let error_param = params
            .get("error")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let state_param = params
            .get("state")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let code_param = params
            .get("code")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        if let Some(err_code) = error_param {
            if let Some(received_state) = state_param.as_deref()
                && received_state != expected_state
            {
                let _ = write_html_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    "OpenMD Google Drive",
                    "OAuth state verification failed. Please return to OpenMD and try again.",
                )
                .await;
                return Err(AuthError::StateMismatch);
            }
            let msg = format!(
                "Google Drive authorization was cancelled or denied ({err_code}). You can close this window."
            );
            let _ = write_html_response(&mut stream, 200, "OK", "OpenMD Google Drive", &msg).await;
            return Err(AuthError::Denied(err_code));
        }

        if state_param.as_deref() != Some(expected_state.as_str()) {
            let _ = write_html_response(
                &mut stream,
                400,
                "Bad Request",
                "OpenMD Google Drive",
                "OAuth state verification failed. Please return to OpenMD and try again.",
            )
            .await;
            return Err(AuthError::StateMismatch);
        }

        let Some(code) = code_param else {
            let _ = write_html_response(
                &mut stream,
                400,
                "Bad Request",
                "OpenMD Google Drive",
                "Missing authorization code. Please return to OpenMD and try again.",
            )
            .await;
            return Err(AuthError::Other(
                "Missing authorization code in OAuth callback".to_string(),
            ));
        };

        let exchange_fut =
            self.exchange_code_and_fetch_profile(&creds, &code, &redirect_uri, &code_verifier);
        let tokens_result = tokio::select! {
            biased;
            _ = wait_for_cancel(&mut cancel_rx) => Err(AuthError::Cancelled),
            res = exchange_fut => res,
        };

        match tokens_result {
            Ok(tokens) => {
                if let Err(err) = self.token_store.save_tokens(&tokens) {
                    let _ = write_html_response(
                        &mut stream,
                        500,
                        "Internal Server Error",
                        "OpenMD Google Drive",
                        "Failed to store OAuth tokens in Keychain. Please return to OpenMD.",
                    )
                    .await;
                    return Err(AuthError::Other(err));
                }
                let _ = write_html_response(
                    &mut stream,
                    200,
                    "OK",
                    "OpenMD Google Drive",
                    "OpenMD Google Drive bağlantısı tamamlandı. Bu pencereyi kapatabilirsiniz. / Google Drive connection complete. You can close this window.",
                )
                .await;
                let email = tokens.email.clone();
                Ok((tokens, email))
            }
            Err(err) => {
                let _ = write_html_response(
                    &mut stream,
                    400,
                    "Bad Request",
                    "OpenMD Google Drive",
                    "Could not complete Google Drive sign-in. Please return to OpenMD for details.",
                )
                .await;
                Err(err)
            }
        }
    }

    pub fn cancel_auth(&self) -> bool {
        let Ok(mut guard) = self.active_auth.lock() else {
            return false;
        };
        if let Some(session) = guard.take() {
            let _ = session.cancel_tx.send(true);
            true
        } else {
            false
        }
    }

    pub async fn get_valid_access_token(&self) -> Result<String, AuthError> {
        if let Some(tokens) = self.token_store.load_tokens().map_err(AuthError::Other)? {
            let now_ms = current_unix_ms();
            if !tokens.access_token.trim().is_empty()
                && now_ms.saturating_add(TOKEN_REFRESH_LEEWAY_MS) < tokens.expires_at_ms
            {
                return Ok(tokens.access_token);
            }
        }

        // Serialize refresh attempts so concurrent callers issue at most one HTTP refresh.
        let _guard = self.refresh_lock.lock().await;

        let tokens = self
            .token_store
            .load_tokens()
            .map_err(AuthError::Other)?
            .ok_or_else(|| AuthError::Other("No stored OAuth tokens".to_string()))?;

        let now_ms = current_unix_ms();
        if !tokens.access_token.trim().is_empty()
            && now_ms.saturating_add(TOKEN_REFRESH_LEEWAY_MS) < tokens.expires_at_ms
        {
            return Ok(tokens.access_token);
        }

        if tokens.refresh_token.trim().is_empty() {
            return Err(AuthError::InvalidGrant("Missing refresh token".to_string()));
        }

        let creds = self.resolve_credentials()?;
        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", "refresh_token"),
            ("refresh_token", &tokens.refresh_token),
            ("client_id", &creds.client_id),
        ];
        if let Some(secret) = creds.client_secret.as_deref().filter(|s| !s.is_empty()) {
            form.push(("client_secret", secret));
        }

        let resp = self
            .http
            .post(&self.endpoints.token_url)
            .timeout(OAUTH_HTTP_TIMEOUT)
            .form(&form)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;
        let refreshed = parse_token_response(resp).await?;

        let expires_in_secs = refreshed.expires_in.unwrap_or(3600).max(0);
        let updated = StoredOAuthTokens {
            refresh_token: refreshed
                .refresh_token
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| tokens.refresh_token.clone()),
            access_token: refreshed.access_token,
            expires_at_ms: current_unix_ms().saturating_add(expires_in_secs.saturating_mul(1000)),
            email: tokens.email,
        };

        self.token_store
            .save_tokens(&updated)
            .map_err(AuthError::Other)?;
        Ok(updated.access_token)
    }

    pub async fn revoke_and_clear_tokens(&self) -> Result<(), String> {
        self.cancel_auth();

        // Serialize with in-flight token refreshes so a concurrent refresh cannot
        // persist renewed tokens into Keychain after sign-out clears them.
        let _refresh_guard = self.refresh_lock.lock().await;

        if let Ok(Some(tokens)) = self.token_store.load_tokens() {
            let token = if !tokens.refresh_token.trim().is_empty() {
                tokens.refresh_token
            } else {
                tokens.access_token
            };
            if !token.trim().is_empty() {
                // Best-effort revoke: local sign-out must still succeed when offline or already revoked.
                let _ = self
                    .http
                    .post(&self.endpoints.revoke_url)
                    .timeout(OAUTH_REVOKE_TIMEOUT)
                    .form(&[("token", token.as_str())])
                    .send()
                    .await;
            }
        }

        self.token_store.delete_tokens()?;
        self.store.clear_all_drive_cache()?;
        Ok(())
    }

    async fn exchange_code_and_fetch_profile(
        &self,
        creds: &OAuthClientCredentials,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<StoredOAuthTokens, AuthError> {
        let mut form: Vec<(&str, &str)> = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", &creds.client_id),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        if let Some(secret) = creds.client_secret.as_deref().filter(|s| !s.is_empty()) {
            form.push(("client_secret", secret));
        }

        let resp = self
            .http
            .post(&self.endpoints.token_url)
            .timeout(OAUTH_HTTP_TIMEOUT)
            .form(&form)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;
        let token_resp = parse_token_response(resp).await?;

        let expires_in_secs = token_resp.expires_in.unwrap_or(3600).max(0);
        let expires_at_ms =
            current_unix_ms().saturating_add(expires_in_secs.saturating_mul(1000));

        let userinfo_resp = self
            .http
            .get(&self.endpoints.userinfo_url)
            .timeout(OAUTH_HTTP_TIMEOUT)
            .bearer_auth(&token_resp.access_token)
            .send()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;
        let status = userinfo_resp.status();
        let userinfo_body = userinfo_resp
            .text()
            .await
            .map_err(|e| AuthError::Network(e.to_string()))?;
        if !status.is_success() {
            return Err(AuthError::Other(format!(
                "Failed to fetch Google userinfo ({status}): {userinfo_body}"
            )));
        }
        let userinfo: UserInfoResponse = serde_json::from_str(&userinfo_body)
            .map_err(|e| AuthError::Other(format!("Invalid userinfo JSON: {e}")))?;

        Ok(StoredOAuthTokens {
            refresh_token: token_resp
                .refresh_token
                .map(|s| s.trim().to_string())
                .unwrap_or_default(),
            access_token: token_resp.access_token,
            expires_at_ms,
            email: userinfo.email.unwrap_or_default().trim().to_string(),
        })
    }
}

pub fn generate_pkce_pair() -> (String, String) {
    let verifier = random_base64url_32();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub fn generate_state() -> String {
    random_base64url_32()
}

fn random_base64url_32() -> String {
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn wait_for_cancel(cancel_rx: &mut watch::Receiver<bool>) {
    loop {
        if *cancel_rx.borrow_and_update() {
            return;
        }
        if cancel_rx.changed().await.is_err() {
            return;
        }
    }
}

fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn encode_query_component(value: &str) -> String {
    utf8_percent_encode(value, QUERY_ENCODE_SET).to_string()
}

fn build_authorization_url(
    base_auth_url: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    code_challenge: &str,
) -> String {
    let sep = if base_auth_url.contains('?') { '&' } else { '?' };
    let params = [
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", DRIVE_OAUTH_SCOPES),
        ("state", state),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ];
    let query = params
        .into_iter()
        .map(|(k, v)| format!("{}={}", encode_query_component(k), encode_query_component(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base_auth_url}{sep}{query}")
}

fn parse_query_params(raw_query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in raw_query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |s: &str| -> String {
            let plus_fixed = s.replace('+', " ");
            percent_decode_str(&plus_fixed)
                .decode_utf8_lossy()
                .to_string()
        };
        map.insert(decode(k), decode(v));
    }
    map
}

async fn parse_token_response(resp: reqwest::Response) -> Result<TokenEndpointResponse, AuthError> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| AuthError::Network(e.to_string()))?;

    if !status.is_success() {
        let parsed_err = serde_json::from_str::<OAuthErrorBody>(&body).ok();
        let err_code = parsed_err
            .as_ref()
            .and_then(|b| b.error.as_deref())
            .unwrap_or("");
        let err_desc = parsed_err
            .as_ref()
            .and_then(|b| b.error_description.as_deref())
            .unwrap_or("");

        if err_code.eq_ignore_ascii_case("invalid_grant")
            || ((status.as_u16() == 400 || status.as_u16() == 401)
                && body.contains("invalid_grant"))
        {
            let detail = if !err_desc.is_empty() {
                format!("invalid_grant: {err_desc}")
            } else {
                "invalid_grant".to_string()
            };
            return Err(AuthError::InvalidGrant(detail));
        }

        let summary = if !err_desc.is_empty() {
            err_desc
        } else if !err_code.is_empty() {
            err_code
        } else {
            &body
        };
        return Err(AuthError::Other(format!(
            "OAuth token endpoint returned HTTP {status}: {summary}"
        )));
    }

    let parsed: TokenEndpointResponse = serde_json::from_str(&body)
        .map_err(|e| AuthError::Other(format!("Invalid OAuth token JSON: {e}")))?;
    if parsed.access_token.trim().is_empty() {
        return Err(AuthError::Other(
            "OAuth token response contained empty access_token".to_string(),
        ));
    }
    Ok(parsed)
}

async fn read_loopback_http_request(stream: &mut TcpStream) -> Option<LoopbackHttpRequest> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 2048];

    let read_result = tokio::time::timeout(LOOPBACK_READ_TIMEOUT, async {
        loop {
            let n = stream.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|w| w == b"\r\n\r\n")
                || buf.windows(2).any(|w| w == b"\n\n")
                || buf.len() >= MAX_LOOPBACK_REQUEST_BYTES
            {
                break;
            }
        }
        Some(())
    })
    .await;

    if read_result.is_err() || buf.is_empty() {
        return None;
    }

    let text = String::from_utf8_lossy(&buf);
    let request_line = text.lines().next()?.trim();
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?;
    let target_no_frag = target.split('#').next().unwrap_or(target);
    let (path, query) = target_no_frag
        .split_once('?')
        .unwrap_or((target_no_frag, ""));

    Some(LoopbackHttpRequest {
        method,
        path: path.to_string(),
        query: query.to_string(),
    })
}

fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

async fn write_html_response(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    title: &str,
    message: &str,
) -> std::io::Result<()> {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
        <style>body{{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;\
        display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0;\
        background:#f7f7f8;color:#1f2328}}main{{max-width:440px;padding:28px 32px;\
        border-radius:12px;background:#fff;box-shadow:0 6px 24px rgba(0,0,0,.08);\
        text-align:center}}h1{{font-size:18px;margin:0 0 10px}}p{{font-size:14px;\
        line-height:1.5;margin:0;color:#57606a}}</style></head>\
        <body><main><h1>{}</h1><p>{}</p></main></body></html>",
        escape_html(title),
        escape_html(title),
        escape_html(message)
    );
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use tempfile::tempdir;
    use tokio::sync::{oneshot, Notify};

    fn make_test_manager(
        endpoints: OAuthEndpoints,
    ) -> (
        tempfile::TempDir,
        Arc<HistoryStore>,
        Arc<MemoryTokenStore>,
        DriveAuthManager,
    ) {
        let dir = tempdir().unwrap();
        let store = Arc::new(HistoryStore::open(dir.path().join("history.db")).unwrap());
        let token_store = Arc::new(MemoryTokenStore::new());
        let manager = DriveAuthManager::new(
            Arc::clone(&store),
            token_store.clone(),
            reqwest::Client::new(),
            endpoints,
        )
        .with_built_in_credentials(None, None);
        (dir, store, token_store, manager)
    }

    #[derive(Clone)]
    struct MockOAuthState {
        token_hits: Arc<AtomicUsize>,
        revoke_hits: Arc<AtomicUsize>,
        captured_token_params: Arc<StdMutex<Option<HashMap<String, String>>>>,
        captured_revoke_body: Arc<StdMutex<Option<String>>>,
        token_delay_ms: u64,
        token_started: Option<Arc<Notify>>,
        token_gate: Option<Arc<Notify>>,
        token_status: u16,
        token_body: String,
        userinfo_email: String,
    }

    impl Default for MockOAuthState {
        fn default() -> Self {
            Self {
                token_hits: Arc::new(AtomicUsize::new(0)),
                revoke_hits: Arc::new(AtomicUsize::new(0)),
                captured_token_params: Arc::new(StdMutex::new(None)),
                captured_revoke_body: Arc::new(StdMutex::new(None)),
                token_delay_ms: 0,
                token_started: None,
                token_gate: None,
                token_status: 200,
                token_body: r#"{"access_token":"access-123","refresh_token":"refresh-456","expires_in":3600,"token_type":"Bearer"}"#.to_string(),
                userinfo_email: "alice@example.com".to_string(),
            }
        }
    }

    async fn read_mock_http_request(stream: &mut TcpStream) -> (String, String, String) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let mut header_end = None;
        let mut content_length = 0usize;

        loop {
            let n = stream.read(&mut tmp).await.unwrap();
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
                for line in headers_str.lines() {
                    if let Some((k, v)) = line.split_once(':')
                        && k.trim().eq_ignore_ascii_case("content-length")
                    {
                        content_length = v.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
            if let Some(end) = header_end
                && buf.len() >= end + content_length
            {
                break;
            }
        }

        let end = header_end.unwrap_or(buf.len());
        let headers_str = String::from_utf8_lossy(&buf[..end]).to_string();
        let body_end = (end + content_length).min(buf.len());
        let body = String::from_utf8_lossy(&buf[end..body_end]).to_string();
        let request_line = headers_str.lines().next().unwrap_or("");
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let target = parts.next().unwrap_or("").to_string();
        (method, target, body)
    }

    async fn spawn_mock_oauth_server(
        state: Arc<StdMutex<MockOAuthState>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let base_url = format!("http://127.0.0.1:{port}");

        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    let (method, target, body) = read_mock_http_request(&mut stream).await;
                    let (path, _query) = target.split_once('?').unwrap_or((&target, ""));

                    let (status, resp_body) = match (method.as_str(), path) {
                        ("POST", "/token") => {
                            let (delay_ms, started, gate, status, resp_body) = {
                                let guard = state.lock().unwrap();
                                guard.token_hits.fetch_add(1, Ordering::SeqCst);
                                *guard.captured_token_params.lock().unwrap() =
                                    Some(parse_query_params(&body));
                                (
                                    guard.token_delay_ms,
                                    guard.token_started.clone(),
                                    guard.token_gate.clone(),
                                    guard.token_status,
                                    guard.token_body.clone(),
                                )
                            };
                            if let Some(started_notify) = started {
                                started_notify.notify_one();
                            }
                            if let Some(gate_notify) = gate {
                                gate_notify.notified().await;
                            }
                            if delay_ms > 0 {
                                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                            }
                            (status, resp_body)
                        }
                        ("GET", "/userinfo") => {
                            let email = state.lock().unwrap().userinfo_email.clone();
                            (200, format!(r#"{{"email":"{email}"}}"#))
                        }
                        ("POST", "/revoke") => {
                            let guard = state.lock().unwrap();
                            guard.revoke_hits.fetch_add(1, Ordering::SeqCst);
                            let captured = if !body.is_empty() { body } else { target };
                            *guard.captured_revoke_body.lock().unwrap() = Some(captured);
                            (200, "{}".to_string())
                        }
                        _ => (404, r#"{"error":"not_found"}"#.to_string()),
                    };

                    let status_text = match status {
                        200 => "OK",
                        400 => "Bad Request",
                        401 => "Unauthorized",
                        _ => "Not Found",
                    };
                    let resp = format!(
                        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        resp_body.len(),
                        resp_body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                    let _ = stream.flush().await;
                });
            }
        });

        (base_url, handle)
    }

    #[test]
    fn test_pkce_s256_challenge_and_state() {
        let (v1, c1) = generate_pkce_pair();
        let (v2, c2) = generate_pkce_pair();
        let s1 = generate_state();
        let s2 = generate_state();

        for val in [&v1, &c1, &v2, &c2, &s1, &s2] {
            assert!(val.len() >= 43, "value too short: {}", val.len());
            assert!(!val.contains('='), "must not contain '=' padding");
            assert!(
                val.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'),
                "must be URL-safe base64: {val}"
            );
        }

        let expected_c1 = URL_SAFE_NO_PAD.encode(Sha256::digest(v1.as_bytes()));
        let expected_c2 = URL_SAFE_NO_PAD.encode(Sha256::digest(v2.as_bytes()));
        assert_eq!(c1, expected_c1);
        assert_eq!(c2, expected_c2);
        assert_ne!(v1, v2);
        assert_ne!(c1, c2);
        assert_ne!(s1, s2);
    }

    #[test]
    fn test_oauth_credential_resolution_builtin_only() {
        let (_dir, store, token_store, unconfigured) = make_test_manager(OAuthEndpoints::default());
        assert_eq!(
            unconfigured.resolve_credentials(),
            Err(AuthError::NotConfigured)
        );
        assert!(!unconfigured.is_configured());

        let manager = DriveAuthManager::new(
            Arc::clone(&store),
            token_store,
            reqwest::Client::new(),
            OAuthEndpoints::default(),
        )
        .with_built_in_credentials(Some("builtin-id".into()), Some("builtin-sec".into()));

        assert!(manager.is_configured());
        assert_eq!(
            manager.resolve_credentials().unwrap(),
            OAuthClientCredentials {
                client_id: "builtin-id".to_string(),
                client_secret: Some("builtin-sec".to_string()),
            }
        );

        let manager_no_secret = DriveAuthManager::new(
            store,
            Arc::new(MemoryTokenStore::new()),
            reqwest::Client::new(),
            OAuthEndpoints::default(),
        )
        .with_built_in_credentials(Some("builtin-id".into()), Some("   ".into()));

        assert_eq!(
            manager_no_secret.resolve_credentials().unwrap(),
            OAuthClientCredentials {
                client_id: "builtin-id".to_string(),
                client_secret: None,
            }
        );
    }

    #[tokio::test]
    async fn test_loopback_oauth_happy_path_with_mock_server() {
        let mock_state = Arc::new(StdMutex::new(MockOAuthState::default()));
        let (base_url, server_handle) = spawn_mock_oauth_server(Arc::clone(&mock_state)).await;

        let endpoints = OAuthEndpoints {
            auth_url: format!("{base_url}/auth"),
            token_url: format!("{base_url}/token"),
            revoke_url: format!("{base_url}/revoke"),
            userinfo_url: format!("{base_url}/userinfo"),
        };

        let (_dir, _store, token_store, base_manager) = make_test_manager(endpoints);
        let manager = base_manager
            .with_built_in_credentials(Some("test-client-id".into()), Some("test-secret".into()));

        let (browser_tx, browser_rx) = oneshot::channel::<(u16, String, String)>();
        let (challenge_tx, challenge_rx) = oneshot::channel::<(String, String)>();

        let (tokens, email) = manager
            .bind_loopback_session(|auth_url| {
                let (_, query) = auth_url
                    .split_once('?')
                    .ok_or_else(|| "missing query string".to_string())?;
                let params = parse_query_params(query);
                assert_eq!(
                    params.get("code_challenge_method").map(String::as_str),
                    Some("S256")
                );
                assert_eq!(
                    params.get("access_type").map(String::as_str),
                    Some("offline")
                );
                assert_eq!(params.get("prompt").map(String::as_str), Some("consent"));
                let scope = params.get("scope").cloned().unwrap_or_default();
                assert!(scope.contains("drive.file"));
                assert!(scope.contains("openid"));
                assert!(scope.contains("email"));

                let redirect_uri = params.get("redirect_uri").cloned().unwrap();
                let state = params.get("state").cloned().unwrap();
                let challenge = params.get("code_challenge").cloned().unwrap();
                let _ = challenge_tx.send((redirect_uri.clone(), challenge));

                let favicon_url = redirect_uri.replace("/oauth/callback", "/favicon.ico");
                let callback_url = format!("{redirect_uri}?code=auth-code-123&state={state}");
                tokio::spawn(async move {
                    let fav_resp = reqwest::get(&favicon_url).await.unwrap();
                    assert_eq!(fav_resp.status().as_u16(), 404);
                    let resp = reqwest::get(&callback_url).await.unwrap();
                    let status = resp.status().as_u16();
                    let content_type = resp
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let body = resp.text().await.unwrap();
                    let _ = browser_tx.send((status, content_type, body));
                });
                Ok(())
            })
            .await
            .unwrap();

        let (browser_status, content_type, browser_body) = browser_rx.await.unwrap();
        assert_eq!(browser_status, 200);
        assert!(content_type.contains("text/html"));
        assert!(browser_body.contains("OpenMD"));

        assert_eq!(email, "alice@example.com");
        assert_eq!(tokens.email, "alice@example.com");
        assert_eq!(tokens.access_token, "access-123");
        assert_eq!(tokens.refresh_token, "refresh-456");
        assert_eq!(token_store.load_tokens().unwrap(), Some(tokens));
        assert!(manager.has_stored_tokens());
        assert!(!manager.is_auth_in_progress());

        let (redirect_uri, sent_challenge) = challenge_rx.await.unwrap();
        let captured = mock_state
            .lock()
            .unwrap()
            .captured_token_params
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert_eq!(
            captured.get("client_secret").map(String::as_str),
            Some("test-secret")
        );
        assert_eq!(
            captured.get("redirect_uri").map(String::as_str),
            Some(redirect_uri.as_str())
        );
        let verifier = captured.get("code_verifier").unwrap();
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            sent_challenge
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_loopback_oauth_rejects_state_mismatch_and_access_denied() {
        let (_dir, _store, token_store, base_manager) = make_test_manager(OAuthEndpoints::default());
        let manager = base_manager
            .with_built_in_credentials(Some("test-client-id".into()), Some("test-secret".into()));

        // 1) State mismatch -> 400 HTML & Err(AuthError::StateMismatch)
        let (resp1_tx, resp1_rx) = oneshot::channel::<(u16, String)>();
        let err1 = manager
            .bind_loopback_session(|auth_url| {
                let (_, query) = auth_url.split_once('?').unwrap();
                let params = parse_query_params(query);
                let redirect_uri = params.get("redirect_uri").unwrap().clone();
                let callback_url = format!("{redirect_uri}?code=abc&state=wrong-state");
                tokio::spawn(async move {
                    let resp = reqwest::get(&callback_url).await.unwrap();
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap();
                    let _ = resp1_tx.send((status, body));
                });
                Ok(())
            })
            .await
            .unwrap_err();
        assert_eq!(err1, AuthError::StateMismatch);
        let (status1, body1) = resp1_rx.await.unwrap();
        assert_eq!(status1, 400);
        assert!(body1.contains("<html"));
        assert_eq!(token_store.load_tokens().unwrap(), None);

        // 2) access_denied -> Err(AuthError::Denied(_))
        let (resp2_tx, resp2_rx) = oneshot::channel::<(u16, String)>();
        let err2 = manager
            .bind_loopback_session(|auth_url| {
                let (_, query) = auth_url.split_once('?').unwrap();
                let params = parse_query_params(query);
                let redirect_uri = params.get("redirect_uri").unwrap().clone();
                let callback_url = format!("{redirect_uri}?error=access_denied");
                tokio::spawn(async move {
                    let resp = reqwest::get(&callback_url).await.unwrap();
                    let status = resp.status().as_u16();
                    let body = resp.text().await.unwrap();
                    let _ = resp2_tx.send((status, body));
                });
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(matches!(err2, AuthError::Denied(ref r) if r == "access_denied"));
        let (_status2, body2) = resp2_rx.await.unwrap();
        assert!(body2.contains("<html"));
        assert_eq!(token_store.load_tokens().unwrap(), None);
    }

    #[tokio::test]
    async fn test_loopback_oauth_cancellation() {
        let (_dir, _store, _token_store, base_manager) = make_test_manager(OAuthEndpoints::default());
        let manager = Arc::new(
            base_manager.with_built_in_credentials(Some("test-client-id".into()), None),
        );

        let (started_tx, started_rx) = oneshot::channel::<()>();
        let manager_clone = Arc::clone(&manager);
        let auth_task = tokio::spawn(async move {
            manager_clone
                .bind_loopback_session(|_auth_url| {
                    let _ = started_tx.send(());
                    Ok(())
                })
                .await
        });

        started_rx.await.unwrap();
        assert!(manager.is_auth_in_progress());
        assert!(manager.cancel_auth());
        assert!(!manager.is_auth_in_progress());

        let result = tokio::time::timeout(Duration::from_secs(2), auth_task)
            .await
            .expect("cancellation must resolve promptly")
            .unwrap();
        assert_eq!(result, Err(AuthError::Cancelled));
        assert!(!manager.cancel_auth());
    }

    #[tokio::test]
    async fn test_token_refresh_serialization_and_invalid_grant() {
        let mock_state = Arc::new(StdMutex::new(MockOAuthState {
            token_delay_ms: 60,
            token_status: 200,
            token_body:
                r#"{"access_token":"fresh-access-1","expires_in":3600,"token_type":"Bearer"}"#
                    .to_string(),
            ..MockOAuthState::default()
        }));
        let (base_url, server_handle) = spawn_mock_oauth_server(Arc::clone(&mock_state)).await;

        let endpoints = OAuthEndpoints {
            auth_url: format!("{base_url}/auth"),
            token_url: format!("{base_url}/token"),
            revoke_url: format!("{base_url}/revoke"),
            userinfo_url: format!("{base_url}/userinfo"),
        };

        let (_dir, _store, token_store, base_manager) = make_test_manager(endpoints);
        let manager = Arc::new(
            base_manager.with_built_in_credentials(
                Some("test-client-id".into()),
                Some("test-secret".into()),
            ),
        );

        token_store
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "orig-refresh".to_string(),
                access_token: "stale-access".to_string(),
                expires_at_ms: 0,
                email: "alice@example.com".to_string(),
            })
            .unwrap();

        let (r1, r2, r3, r4) = tokio::join!(
            manager.get_valid_access_token(),
            manager.get_valid_access_token(),
            manager.get_valid_access_token(),
            manager.get_valid_access_token(),
        );
        assert_eq!(r1.unwrap(), "fresh-access-1");
        assert_eq!(r2.unwrap(), "fresh-access-1");
        assert_eq!(r3.unwrap(), "fresh-access-1");
        assert_eq!(r4.unwrap(), "fresh-access-1");
        assert_eq!(
            mock_state.lock().unwrap().token_hits.load(Ordering::SeqCst),
            1
        );

        let persisted = token_store.load_tokens().unwrap().unwrap();
        assert_eq!(persisted.access_token, "fresh-access-1");
        assert_eq!(persisted.refresh_token, "orig-refresh");
        assert!(persisted.expires_at_ms > 0);

        // Now force expiry and configure invalid_grant 400 response
        token_store
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "revoked-refresh".to_string(),
                access_token: "stale-access".to_string(),
                expires_at_ms: 0,
                email: "alice@example.com".to_string(),
            })
            .unwrap();
        {
            let mut guard = mock_state.lock().unwrap();
            guard.token_delay_ms = 0;
            guard.token_status = 400;
            guard.token_body =
                r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#
                    .to_string();
        }

        let err = manager.get_valid_access_token().await.unwrap_err();
        assert!(matches!(err, AuthError::InvalidGrant(_)));

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_revoke_and_clear_tokens() {
        let mock_state = Arc::new(StdMutex::new(MockOAuthState::default()));
        let (base_url, server_handle) = spawn_mock_oauth_server(Arc::clone(&mock_state)).await;

        let endpoints = OAuthEndpoints {
            auth_url: format!("{base_url}/auth"),
            token_url: format!("{base_url}/token"),
            revoke_url: format!("{base_url}/revoke"),
            userinfo_url: format!("{base_url}/userinfo"),
        };

        let (_dir, store, token_store, manager) = make_test_manager(endpoints);
        store
            .set_drive_setting_str("drive_account_email", "alice@example.com")
            .unwrap();
        store
            .set_drive_setting_str("drive_enabled", "true")
            .unwrap();
        token_store
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "revokable-refresh-token".to_string(),
                access_token: "revokable-access".to_string(),
                expires_at_ms: 9_999_999_999_999,
                email: "alice@example.com".to_string(),
            })
            .unwrap();

        manager.revoke_and_clear_tokens().await.unwrap();

        assert_eq!(
            mock_state.lock().unwrap().revoke_hits.load(Ordering::SeqCst),
            1
        );
        let revoke_body = mock_state
            .lock()
            .unwrap()
            .captured_revoke_body
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert!(revoke_body.contains("revokable-refresh-token"));
        assert_eq!(token_store.load_tokens().unwrap(), None);
        assert!(!manager.has_stored_tokens());
        assert_eq!(
            store.get_drive_setting_str("drive_account_email").unwrap(),
            ""
        );
        assert_eq!(
            store.get_drive_setting_str("drive_enabled").unwrap(),
            "false"
        );

        server_handle.abort();
    }

    #[tokio::test]
    async fn test_revoke_waits_for_inflight_refresh_and_does_not_resurrect_tokens() {
        let token_started = Arc::new(Notify::new());
        let token_gate = Arc::new(Notify::new());
        let mock_state = Arc::new(StdMutex::new(MockOAuthState {
            token_started: Some(Arc::clone(&token_started)),
            token_gate: Some(Arc::clone(&token_gate)),
            token_status: 200,
            token_body: r#"{"access_token":"fresh-access-race","refresh_token":"fresh-refresh-race","expires_in":3600,"token_type":"Bearer"}"#.to_string(),
            ..MockOAuthState::default()
        }));
        let (base_url, server_handle) = spawn_mock_oauth_server(Arc::clone(&mock_state)).await;

        let endpoints = OAuthEndpoints {
            auth_url: format!("{base_url}/auth"),
            token_url: format!("{base_url}/token"),
            revoke_url: format!("{base_url}/revoke"),
            userinfo_url: format!("{base_url}/userinfo"),
        };

        let (_dir, store, token_store, base_manager) = make_test_manager(endpoints);
        let manager = Arc::new(
            base_manager.with_built_in_credentials(
                Some("test-client-id".into()),
                Some("test-secret".into()),
            ),
        );

        store
            .set_drive_setting_str("drive_account_email", "alice@example.com")
            .unwrap();
        store
            .set_drive_setting_str("drive_enabled", "true")
            .unwrap();
        token_store
            .save_tokens(&StoredOAuthTokens {
                refresh_token: "stale-refresh-race".to_string(),
                access_token: "stale-access-race".to_string(),
                expires_at_ms: 0,
                email: "alice@example.com".to_string(),
            })
            .unwrap();

        let refresh_manager = Arc::clone(&manager);
        let refresh_task =
            tokio::spawn(async move { refresh_manager.get_valid_access_token().await });

        // Wait until get_valid_access_token() holds refresh_lock inside POST /token.
        token_started.notified().await;

        let revoke_finished = Arc::new(AtomicBool::new(false));
        let revoke_finished_flag = Arc::clone(&revoke_finished);
        let revoke_manager = Arc::clone(&manager);
        let revoke_task = tokio::spawn(async move {
            let res = revoke_manager.revoke_and_clear_tokens().await;
            revoke_finished_flag.store(true, Ordering::SeqCst);
            res
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !revoke_finished.load(Ordering::SeqCst),
            "revoke_and_clear_tokens must wait for active refresh_lock before deleting tokens"
        );

        token_gate.notify_one();

        let refresh_res = refresh_task.await.unwrap();
        assert_eq!(refresh_res.unwrap(), "fresh-access-race");
        revoke_task.await.unwrap().unwrap();

        assert_eq!(
            token_store.load_tokens().unwrap(),
            None,
            "in-flight refresh must not resurrect tokens after revoke_and_clear_tokens"
        );
        assert!(!manager.has_stored_tokens());
        assert_eq!(
            store.get_drive_setting_str("drive_account_email").unwrap(),
            ""
        );
        assert_eq!(
            store.get_drive_setting_str("drive_enabled").unwrap(),
            "false"
        );
        assert_eq!(
            mock_state.lock().unwrap().revoke_hits.load(Ordering::SeqCst),
            1
        );
        let revoke_body = mock_state
            .lock()
            .unwrap()
            .captured_revoke_body
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        assert!(revoke_body.contains("fresh-refresh-race"));

        server_handle.abort();
    }
}
