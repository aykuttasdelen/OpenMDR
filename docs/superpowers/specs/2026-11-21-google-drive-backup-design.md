# Google Drive Bulut Yedekleme (Aşama 2) — Tasarım Dokümanı

**Tarih:** 2026-11-21  
**Proje:** OpenMD (`Rust / Tauri v2 + React 19 + TypeScript + CodeMirror 6`)  
**Kapsam:** Aşama 2 — Yalnızca Tauri masaüstü uygulaması için tek yönlü otomatik Google Drive yedekleme (`Yerel → Google Drive`), güvenli OAuth 2.0 (PKCE + Keychain) bağlantısı ve Drive'dan isteğe bağlı geri yükleme (`Drive → Yerel`). Tarayıcı/web modu kapsam dışındadır.  
**Boyutlandırma:** Orta-büyük ölçekli bağımsız bir özellik (OAuth akışı, yerel veritabanı migrasyonu, arka plan senkronizasyon motoru, kapanış flush mekanizması ve geri yükleme arayüzü) olduğu için tam tasarım dokümanı olarak hazırlanmıştır.

---

## 1. Amaç ve Neden

Aşama 1 ile eklenen yerel sürüm geçmişi (`history.db`), yanlış düzenlemelere ve yanlışlıkla dosya/klasör silmeye karşı cihaz içi koruma sağlar. Ancak tüm veriler yerel diskte kaldığı için cihaz kaybı, SSD arızası veya yeni bir Mac'e geçiş durumlarında tek başına yeterli değildir.

Aşama 2 şu ihtiyacı çözer:
1. OpenMD kullanıcılarının Markdown/MDX notları **Google Drive** hesaplarında görünür bir `OpenMD/` klasörü altında düzenli ve okunabilir biçimde sürekli yedeklenir.
2. Kullanıcı hatasını en aza indirmek için **iki yönlü çakışmalı senkronizasyon yerine tek yönlü otomatik yedekleme (`Yerel → Drive`) + isteğe bağlı geri yükleme (`Drive → Yerel`)** modeli uygulanır.
3. Yeni bir cihaza geçildiğinde kullanıcı aynı Google hesabıyla bağlanıp Drive'daki tüm not ağacını canlı görüntüleyebilir, tek dosya ya da tüm klasör ağacı halinde güvenle geri yükleyebilir.

---

## 2. Hedefler, Hedef Dışındakiler ve Anti-Hedefler

### Hedefler
- **Tek yönlü güvenli yedekleme:** Yerel disk her zaman yetkili kaynak (source of truth) kalır. Yerelde silinen dosyalar Google Drive'dan **asla otomatik silinmez**.
- **Anlaşılır Drive klasör hiyerarşisi:** Yedekler Drive kökünde görünür `OpenMD/` klasörü altında kullanıcının klasör hiyerarşisini koruyarak saklanır (ör. `OpenMD/Documents/Notlarim/Proje/plan.md`). Aynı isimli dosyalar çakışmaz ve notlar Google Drive arayüzünden doğrudan okunabilir.
- **Hassas olmayan (non-sensitive) OAuth kapsamı:** `https://www.googleapis.com/auth/drive.file openid email` kapsamları kullanılır; böylece Google'ın tüm Drive'ı okuyan ağır CASA güvenlik denetimine girmeden, yalnızca OpenMD'nin oluşturduğu klasör/dosyalara erişilir.
- **Tek tıkla bağlantı + BYO desteği:** Varsayılan olarak derleme zamanı gömülen OpenMD Masaüstü OAuth istemcisi ile tek tıkta bağlanılır; ayrıca Ayarlar > Gelişmiş altında kullanıcının kendi Google Cloud OAuth Desktop istemcisini girebileceği ("Kendi OAuth İstemcini Kullan / BYO") seçenek sunulur.
- **Güvenli yerel mimari:** OAuth 2.0 PKCE + yerel loopback (`127.0.0.1:<port>`) akışı ve Drive REST v3 çağrıları tamamen Rust tarafında çalışır; token'lar macOS Keychain'de (`keyring`) saklanır ve `tauri.conf.json` içindeki sıkı CSP hiç gevşetilmez.
- **Akıllı zamanlama & düşük ağ yükü:**
  - Yerel dosya kaydedildikten sonra **hareketsizlik süresi** (varsayılan `30 saniye`, aralık `10–300 sn`) dolduğunda veya kesintisiz düzenlemede **periyodik aralıkta** (varsayılan `5 dakika`, aralık `1–60 dk`) arka planda otomatik yedeklenir.
  - Yükleme öncesi `SHA-256` özet + uzak yol kontrolü yapılır; içerik ve yol değişmemişse gereksiz Drive API isteği atılmaz.
  - Çevrimdışı durumda bekleyen dosyalar `history.db` kuyruğunda kalıcı tutulur ve üstel geri çekilme (exponential backoff) ile ağ gelince yüklenir.
- **Kapanışta güvenli tamamlama (Quit Flush):** Uygulama kapatılırken bekleyen yedekler varsa kullanıcı bilgilendirilir, en fazla `10 saniye` bekleyerek yükleme tamamlanır veya kullanıcı isterse `"Beklemeden Kapat"` ile hemen çıkabilir.
- **Sıfırdan cihaz kurtarma (`DriveRestoreModal`):** Yerel veritabanı boş olsa bile Drive'daki `OpenMD/` alt ağacını canlı listeler; arama, içerik önizleme, yeni sekmede açma, seçilen klasöre çakışmasız geri yükleme ve tüm yedekleri toplu indirme sunar.
- **İki dilli arayüz:** Türkçe ve İngilizce (`src/i18n/translations.ts`).

### Hedef Dışındakiler
- **Web / tarayıcı sürümü desteği:** Tarayıcı modunda (`!isTauri()`) Google Drive UI ve komutları gizlenir; bu özellik yalnızca Tauri masaüstü uygulaması içindir.
- **İki yönlü senkronizasyon:** Google Drive üzerinde başka bir araçla düzenlenen dosyaları otomatik yerele indirme veya 3 yollu birleştirme.
- **İkili dosyalar / `assets/` görselleri:** Aşama 2 yalnızca `.md`, `.mdx`, `.markdown` metin belgelerini (en fazla `10 MiB`) yedekler.
- **Google Drive sürüm geçmişi gezgini:** Sürüm farkı/geri alma Aşama 1 yerel `history.db` üzerinden yönetilir; Drive her dosya için güncel yedek kopyasını tutar.
- **Harici dosya izleyici (file watcher):** OpenMD dışındaki uygulamaların yaptığı değişiklikleri izlemek yerine OpenMD yazma/oluşturma/taşıma/kurtarma yaşam döngüsü ve isteğe bağlı `"Şimdi Yedekle"` çalışma alanı taraması kullanılır.

### Anti-Hedefler
- **Yerel kaydı engelleme / yavaşlatma:** İnternet kesintisi, token hatası veya Drive API yavaşlığı hiçbir koşulda yerel `write_file`, `create_file`, `rename_file` veya `save_file_dialog` işlemlerini başarısız kılmaz ya da bekletmez.
- **Token veya secret sızdırma:** Refresh/access token veya BYO `client_secret` hiçbir zaman `localStorage`, WebView JS belleği, git deposu ya da düz metin config dosyalarına yazılmaz; IPC cevaplarında frontend'e geri döndürülmez.
- **Kapanışta sonsuz kilitlenme:** Uygulamadan çıkarken çevrimdışı veya yavaş ağ nedeniyle kullanıcı süresiz kilitlenmemelidir (maksimum 10 sn zaman aşımı + anında `"Beklemeden Kapat"` düğmesi).
- **Sessiz dosya ezme / path traversal:** Drive'dan geri yükleme mevcut yerel dosyaların üzerine asla sessizce yazmaz; her yol bileşeni doğrulanır ve `resolve_available_path` ile çakışmasız isim üretilir.

---

## 3. Değerlendirilen Yaklaşımlar ve Seçim

| Kriter | Yaklaşım A: Tamamen Rust (Tauri Backend) Motoru *(Seçilen)* | Yaklaşım B: Hibrit (Rust Auth + JS Drive API) |
|---|---|---|
| **Güvenlik & CSP** | Token'lar Keychain + Rust belleğinde kalır; `tauri.conf.json` CSP değişmez | Access token JS belleğine çıkar; CSP `connect-src`'e `googleapis.com` eklenir |
| **Kapanış Flush Güvenilirliği** | Tek bir `tokio` motoru kuyruğu atomik boşaltır ve timeout yönetir | JS timer/kuyruk ile Rust `app:window-closing` arası senkronizasyon karmaşası |
| **Dosya Yaşam Döngüsü** | `fs_ops.rs` & `history_ops.rs` doğrudan Rust içinde kuyruğa ekler | Her write/rename sonrası Frontend event köprüsü gerekir |
| **Test Edilebilirlik** | Mock `TokenStore` + yerel `tokio` HTTP sunucusuyla uçtan uca `cargo test` | Ağ/timer testleri dağınık |
| **Bağımlılık Etkisi** | `reqwest` (`rustls-tls`), `keyring`, `rand` eklenir | `reqwest` gerekmez |

**Baştan Elenenler:** Harici `rclone` CLI veya Google Drive for Desktop klasör aynalama (kullanıcıdan ek sistem yazılımı kurmasını gerektirdiği için tek tık deneyimine aykırıdır).

**Seçilen Yaklaşım:** **Yaklaşım A (Tamamen Rust Tabanlı Motor)**. Kullanıcı tarafından güvenlik, tek bütünlük ve kapanış güvenilirliği nedeniyle onaylanmıştır.

---

## 4. Mimari ve Teknik Tasarım

### 4.1 Yeni Bağımlılıklar (`src-tauri/Cargo.toml`) ve CSP
- `reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }` — Sistem OpenSSL bağımlılığı olmayan saf Rust TLS HTTP istemcisi.
- `keyring = { version = "3", features = ["apple-native"] }` — macOS Keychain native erişimi (birim/entegrasyon testlerinde `TokenStore` trait'i bellek içi mock ile değiştirilir).
- `rand = "0.9"` — Kriptografik CSPRNG ile OAuth `state` ve PKCE `code_verifier` üretimi için.
- **HTTP loopback dinleyici:** Ek HTTP sunucu crate'i eklenmez; mevcut `tokio::net::TcpListener` (`127.0.0.1:0`) üzerinden tek kullanımlık HTTP/1.1 callback okunup yanıtlanır.
- **`src-tauri/tauri.conf.json`:** Mevcut sıkı CSP (`connect-src 'self' ipc: http://ipc.localhost ws://localhost:5173`) **değiştirilmez**. `capabilities/default.json` içindeki mevcut `opener:default` izni harici tarayıcı açılışı (`OpenerExt`) için yeterlidir.

### 4.2 Backend Modül Yapısı (`src-tauri/src/`)

| Dosya | Sorumluluk |
|---|---|
| `history_store.rs` | `PRAGMA user_version` 1 → 2 migrasyonu; `drive_folders`, `drive_files`, `settings` içindeki `drive_*` anahtarları ve SQLite CRUD metotları |
| `drive_auth.rs` | `TokenStore` trait (`KeychainTokenStore` / `MemoryTokenStore`), OAuth 2.0 PKCE + loopback sunucusu, iptal token'ı, istemci kimliği çözümleme, tekil token yenileme (`auth_lock`), revoke/disconnect |
| `drive_client.rs` | Yapılandırılabilir base URL'lere sahip Google OAuth/Drive REST v3 HTTP istemcisi (`oauth_token_url`, `oauth_revoke_url`, `userinfo_url`, `drive_api_base_url`, `drive_upload_base_url`), klasör bul/oluştur, multipart upload, media PATCH, rename/move PATCH (`addParents`/`removeParents`), sayfalı listeleme, indirme |
| `drive_sync.rs` | `DriveManager` (Tauri `DriveState`), `compute_remote_path`, dosya uygunluk kontrolleri (`.md`/`.mdx`/`.markdown`, `<= 10 MiB`), tekil `sync_lock`, arka plan `tokio` işçisi (hareketsizlik + periyodik aralık + üstel backoff), `flush_before_quit`, çalışma alanı ilk taraması, `drive:status-changed` event yayını |
| `commands/drive_ops.rs` | Frontend ile konuşan tüm Tauri IPC komutları ve `drive_restore_file` çakışmasız disk kurtarma |

### 4.3 Veritabanı Şeması ve Migrasyon (`history.db`, `PRAGMA user_version = 2`)

**Önemli Düzeltme (v1 → v2):** Aşama 1'deki `HistoryStore::open` (`src-tauri/src/history_store.rs`) şu anda açılışta koşulsuz `PRAGMA user_version = 1;` çalıştırmaktadır. Aşama 2'de bu yapı sürüm kontrollü migrasyona çevrilir:
1. `journal_mode = WAL`, `foreign_keys = ON`, `busy_timeout = 5000` ayarlanır.
2. `let version: i64 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?;` okunur.
3. `version < 1` ise tek transaction içinde v1 tabloları (`settings`, `tracked_files`, `snapshots`, `idx_snapshots_file_created`) + varsayılan v1 ayarları oluşturulup `PRAGMA user_version = 1;` yazılır.
4. `version < 2` ise tek transaction içinde v2 tabloları + varsayılan `drive_*` ayarları eklenip `PRAGMA user_version = 2;` yazılır. Mevcut v1 `snapshots` ve `settings` satırlarına hiç dokunulmaz.

*(Zaman damgası birimi: `history_store.rs` v1 konvansiyonuna birebir uyumlu olarak tüm SQLite zaman damgaları **Unix milisaniyesi (`i64`)** cinsindendir.)*

```sql
CREATE TABLE IF NOT EXISTS drive_folders (
    remote_folder_path TEXT PRIMARY KEY, -- '' = OpenMD kökü, 'Documents/Notlarim' vb.
    drive_folder_id TEXT NOT NULL,
    parent_drive_id TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS drive_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    local_path TEXT NOT NULL UNIQUE,
    remote_path TEXT NOT NULL UNIQUE,    -- 'OpenMD/' altındaki göreli yol: 'Documents/plan.md'
    drive_file_id TEXT,                  -- Google Drive üzerindeki kalıcı file.id
    drive_parent_id TEXT,                -- Son bilinen Drive üst klasör id'si
    last_synced_hash TEXT,               -- Son başarıyla yüklenen UTF-8 içeriğin SHA-256 özeti
    last_synced_remote_path TEXT,        -- Son başarıyla yüklenen uzak yol (rename tespiti için)
    last_synced_at INTEGER,              -- Unix ms
    dirty INTEGER NOT NULL DEFAULT 1,    -- 1 = yükleme/taşıma bekliyor, 0 = güncel
    dirty_since INTEGER,                 -- İlk kuyruğa giriş zamanı (Unix ms)
    retry_count INTEGER NOT NULL DEFAULT 0,
    next_retry_at INTEGER NOT NULL DEFAULT 0, -- Backoff bitiş zamanı (Unix ms)
    last_error TEXT,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_drive_files_dirty
    ON drive_files(dirty, next_retry_at, updated_at);
```

- **Varsayılan `settings` anahtarları (`INSERT OR IGNORE`):**
  - `drive_enabled`: `'false'` (OAuth bağlantısı başarıyla tamamlanınca otomatik `'true'` yapılır)
  - `drive_sync_interval_minutes`: `'5'` (geçerli aralık `1..=60`)
  - `drive_idle_delay_seconds`: `'30'` (geçerli aralık `10..=300`)
  - `drive_account_email`: `''`
  - `drive_root_folder_id`: `''` (Drive üzerindeki `OpenMD` kök klasör ID'si)
  - `drive_custom_client_id`: `''`
  - `drive_last_sync_at`: `'0'` (Unix ms; `0` = henüz senkronizasyon yok)

### 4.4 OAuth 2.0 + PKCE + Keychain Tasarımı (`drive_auth.rs`)

- **Google Cloud Console yapılandırması:** OAuth Client türü **Desktop app**. İstenen kapsamlar: `https://www.googleapis.com/auth/drive.file openid email` (tümü non-sensitive).
- **Uç noktalar (varsayılanlar):**
  - Yetkilendirme: `https://accounts.google.com/o/oauth2/v2/auth`
  - Token değişimi / yenileme: `https://oauth2.googleapis.com/token`
  - İptal (revoke): `https://oauth2.googleapis.com/revoke`
  - Hesap e-postası: `https://openidconnect.googleapis.com/v1/userinfo`
- **İstemci kimliği çözümleme sırası:**
  1. **Özel (BYO) istemci:** `history.db` içinde `drive_custom_client_id` boş değilse bu ID + Keychain'deki opsiyonel `"google-drive-custom-client-secret"` değeri kullanılır.
  2. **Yerleşik OpenMD istemcisi:** Derleme sırasında `OPENMD_GOOGLE_CLIENT_ID` ortam değişkeni verilmişse `option_env!("OPENMD_GOOGLE_CLIENT_ID")` + opsiyonel `option_env!("OPENMD_GOOGLE_CLIENT_SECRET")` kullanılır (git deposuna hiçbir client ID veya secret sabit kodlanmaz).
  3. İkisi de yoksa `drive_connect` `"OAUTH_CLIENT_NOT_CONFIGURED"` hatası döner ve UI kullanıcıya doğrudan Ayarlar altındaki **Özel OAuth İstemcisi (BYO)** panelini açtırır.
- **PKCE token değişiminde `client_secret` davranışı:** GCP Desktop app istemcileri genellikle `client_id` ile birlikte bir `client_secret` üretir (RFC 8252 uyarınca gizli sayılmayan public client sabiti). `client_secret` boş değilse form-urlencoded POST gövdesine `client_secret=...` alanı da eklenir; boşsa yalnızca PKCE `code_verifier` gönderilir.
- **Loopback akışı (`drive_connect`):**
  1. Varsa önceki devam eden auth oturumunu iptal kanalından (`tokio::sync::watch`) kapatır.
  2. `127.0.0.1:0` adresine `TcpListener` bağlanır; `redirect_uri = http://127.0.0.1:<port>/oauth/callback` üretilir.
  3. 32 rastgele bayt (URL-safe base64 no-pad) ile `state` ve `code_verifier` üretilir; `code_challenge = BASE64URL(SHA256(code_verifier))`, `code_challenge_method = S256`.
  4. `access_type=offline&prompt=consent` parametreleriyle yetkilendirme URL'i oluşturulur ve `app.opener().open_url(auth_url, None::<&str>)` ile sistem tarayıcısı açılır.
  5. En fazla `180 saniye` boyunca `/oauth/callback` isteği beklenir (`drive_cancel_auth` ile hemen iptal edilebilir).
  6. Gelen HTTP GET satırından query parametreleri ayrıştırılır:
     - `error=access_denied` (veya başka `error`): Tarayıcıya temiz bilgi/hata HTML sayfası döner ve `Err("AUTH_DENIED")` (veya hata kodu) ile sonlanır.
     - `state` beklenen ile eşleşmezse `400 Bad Request` HTML döner ve `Err("AUTH_STATE_MISMATCH")` ile sonlanır.
     - Geçerli `code` varsa `POST /token` ile `access_token`, `refresh_token`, `expires_in` alınır; ardından `GET /userinfo` ile `email` çekilir. Tarayıcıya *"OpenMD Google Drive bağlantısı tamamlandı. Bu pencereyi kapatabilirsiniz."* HTML sayfası döner.
- **Keychain saklama (`TokenStore` trait):**
  - Servis adı: `"com.openmd.editor"`
  - OAuth token hesabı: `"google-drive-oauth"` — JSON içeriği: `{ "refreshToken": "...", "accessToken": "...", "expiresAtMs": 1700000000000, "email": "user@example.com" }`.
  - Özel BYO secret hesabı: `"google-drive-custom-client-secret"` — düz string (yalnızca ayarlanmışsa).
- **Tekil Token Yenileme ve İptal:**
  - `DriveAuthManager` içinde `refresh_lock: tokio::sync::Mutex<()>` bulunur.
  - Token istemeden önce `now_ms + 60_000 >= expires_at_ms` ise kilit altında tek bir `grant_type=refresh_token` isteği yapılır (Google yeni `refresh_token` döndürmezse eskisi korunur).
  - Sunucu HTTP `400/401` ile `"invalid_grant"` döndürürse yerel oturum `auth_required` durumuna geçirilir, otomatik arka plan senkronizasyonu duraklatılır ve kullanıcıya *"Google Drive oturumunun süresi doldu; yeniden bağlanın"* bildirilir.
- **OAuth istemci değişimi ve Bağlantıyı Kes (`drive_disconnect` / `drive_save_custom_oauth` / `drive_clear_custom_oauth`):**
  - `drive.file` kapsamı izinleri OAuth `client_id` bazlı tuttuğundan, BYO istemci kaydedildiğinde veya silindiğinde etkin istemci kimliği değişirse mevcut bağlantı temizce kesilir (veya yeni bağlantıda yeni `OpenMD/` kök klasörü çözümlenir).
  - `drive_disconnect`: Aktif auth dinleyicisini iptal eder → arka plan sync kilidinin mevcut dosya adımını bitirmesini bekler → en iyi çabayla `POST /revoke?token=...` gönderir → Keychain `"google-drive-oauth"` kaydını siler → `drive_files` ve `drive_folders` tablolarını boşaltır → `drive_account_email = ''`, `drive_root_folder_id = ''`, `drive_enabled = 'false'` yapar ve `drive:status-changed` yayar.

### 4.5 Klasör Yapısı ve Deterministik Yol Eşleme (`compute_remote_path`)

Tüm yedekler Google Drive üzerinde görünür **`OpenMD`** kök klasörünün altında tutulur (`application/vnd.google-apps.folder`, `trashed = false`).

**Yol eşleme kuralı (`compute_remote_path(local_path: &Path, home_dir: Option<&Path>) -> Result<String, String>`):**
1. Yalnızca uzantısı küçük/büyük harf duyarsız `.md`, `.mdx`, `.markdown` olan dosyalar yedeklenmeye uygundur.
2. `local_path` bileşenlerinde `.` veya `..` bulunamaz.
3. Normalizasyon sonrasında `local_path` kullanıcının ev dizini (`home_dir`, ör. `/Users/aykut`) altındaysa `home_dir/` öneki çıkarılır:
   - `/Users/aykut/Documents/Notlarim/Proje/plan.md` → `Documents/Notlarim/Proje/plan.md`  
     *(Google Drive'da görünüm: `OpenMD/Documents/Notlarim/Proje/plan.md`)*
   - Başka bir Mac'te kullanıcı adı `/Users/aykuttasdelen/Documents/Notlarim/...` olsa bile ev dizini öneki atıldığı için uzak yol aynı kalır!
4. `local_path`, `home_dir` dışındaysa (ör. `/Volumes/HariciDisk/Notlar/fikir.md` veya `/tmp/not.md`):
   - Kök ayırıcı (varsa Windows sürücü harfi `C:` → `C`) temizlenir ve `_external/` altına konur:  
     `_external/Volumes/HariciDisk/Notlar/fikir.md`  
     *(Drive'da: `OpenMD/_external/Volumes/HariciDisk/Notlar/fikir.md`)*
5. Tüm uzak yollar POSIX `/` ayırıcı ile saklanır ve en az bir geçerli dosya adı içerir.

### 4.6 Drive REST v3 İstemcisi (`drive_client.rs`)

- **Özel sorgu kaçışı:** Drive `q` sorgu dizgesindeki klasör/dosya adlarında `\` ve `'` karakterleri önce `\\` ve `\'` olarak kaçılır (`escape_drive_query_literal`).
- **Klasör bul veya oluştur (`ensure_folder`):**
  1. Önce `drive_folders` yerel SQLite önbelleğine bakar (`remote_folder_path`).
  2. Yoksa Drive `GET /drive/v3/files` ile arar:
     - `OpenMD` kökü için: `name = 'OpenMD' and mimeType = 'application/vnd.google-apps.folder' and 'root' in parents and trashed = false` (`orderBy=modifiedTime desc&pageSize=1&fields=files(id,name)`).
     - Alt klasör için: `name = '<escaped>' and mimeType = 'application/vnd.google-apps.folder' and '<parentId>' in parents and trashed = false`.
  3. Yine yoksa `POST /drive/v3/files` ile `{ "name": "<segment>", "mimeType": "application/vnd.google-apps.folder", "parents": ["<parentId>"] }` oluşturur ve dönen `id` değerini `drive_folders` tablosuna yazar.
  4. Önbellekteki `drive_folder_id` ile işlem HTTP `404 Not Found` dönerse yerel `drive_folders` önbelleğinden ilgili klasör ve alt önekleri silinir, `drive_root_folder_id` (kök 404 ise) temizlenir ve klasör zinciri yeniden oluşturulur (**kendi kendini onarma / self-healing**).
- **Dosya yükleme / güncelleme / taşıma:**
  - Yüklenen dosyalar Drive'da `mimeType: "text/markdown; charset=utf-8"` ile saklanır.
  - **Yeni oluşturma (veya `drive_file_id == None`):**
    1. Önce hedef `parent_id` altında aynı isimli aktif dosya var mı diye aranır (`name = '<escaped>' and '<parentId>' in parents and mimeType != 'application/vnd.google-apps.folder' and trashed = false`).
    2. Varsa bulunan `file.id` üzerine içerik `PATCH` yapılır (böylece başka bir bilgisayarda zaten yedeklenmiş aynı dosyaya ikinci kopya oluşturulmaz).
    3. Yoksa `POST /upload/drive/v3/files?uploadType=multipart&fields=id,name,parents,modifiedTime` ile metadata JSON + UTF-8 dosya içeriği tek istekte yüklenir.
  - **Var olan dosyayı güncelleme (`drive_file_id == Some(id)`):**
    - Yol değişmemişse (`remote_path == last_synced_remote_path`): `PATCH /upload/drive/v3/files/{fileId}?uploadType=media&fields=id,modifiedTime` ile içerik yüklenir.
    - Dosya adı veya klasörü değişmişse (`rename_file`): önce `PATCH /drive/v3/files/{fileId}?addParents={newParentId}&removeParents={oldParentId}&fields=id,name,parents` (Drive v3 kuralı: `parents` gövdede değil query parametresiyle taşınır; ad değişmişse JSON gövdesine `{ "name": "<yeniAd>" }` konur) ile taşınır, ardından içerik hash'i de değişmişse `uploadType=media` ile içerik güncellenir.
    - Herhangi bir `PATCH` HTTP `404` dönerse `drive_file_id` ve `drive_parent_id` `NULL` yapılarak yeniden oluşturma yoluna düşülür.
- **Uzak dosyaları listeleme (`list_files_under_root`):**
  - `OpenMD/` kök klasör ID'si bulunur; bulunamazsa boş liste döner (henüz yedek yok).
  - Genişlik öncelikli (BFS) olarak `OpenMD` ve alt klasör ID'leri `q="'<folderId>' in parents and trashed = false"` + `nextPageToken` sayfalamasıyla taranır (`fields=nextPageToken,files(id,name,mimeType,size,modifiedTime,parents)`), güvenlik için en fazla `5.000` öğe sınırıyla klasör yolu (`Documents/Notlarim/plan.md`) reconstruct edilir ve yalnızca `.md`/`.mdx`/`.markdown` dosyaları `modifiedTime DESC` sırada döndürülür.
- **Dosya içeriği indirme (`download_file_content`):**
  - İndirme öncesi/sırasında boyut `<= 10 MiB` (`10 * 1024 * 1024`) doğrulanır (`GET /drive/v3/files/{fileId}?alt=media`).

## 4.7 Yaşam Döngüsü Hook'ları ve Senkronizasyon Motoru (`drive_sync.rs`)

**Yerel yazma / taşıma / silme entegrasyonu:**
*(Tümü en iyi çaba / best-effort çalışır; SQLite veya bellek kanalı hatası yerel disk işlemlerini asla engellemez.)*
1. `fs_ops::write_file`, `dialogs::save_file_dialog`, `fs_ops::create_file`, `history_ops::restore_snapshot_inner`, `history_ops::recover_file_inner`:
   - Dosya diske başarıyla yazıldıktan sonra `drive_manager.enqueue_local_file(&path)` çağrılır.
   - Hesap bağlı değilse **veya** `drive_enabled == false` (otomatik yedekleme kullanıcı tarafından duraklatılmışsa) yeni dosya/düzenleme arka plan kuyruğuna eklenmez (`pendingCount` kendiliğinden büyümez).
   - Bağlı ve `drive_enabled == true` ise ve uzantı uygunsa `drive_files` içinde satır eklenir/güncellenir (`dirty = 1`, `dirty_since = COALESCE(dirty_since, now_ms)`, `next_retry_at = 0`), `idle_deadline = now + idle_delay_seconds` ileri alınır ve arka plan işçisine bildirim (`Notify`) gönderilir.
2. `fs_ops::rename_file`:
   - `tokio::fs::rename` başarılı olduktan sonra bağlı hesap varsa (`connected == true`), `drive_manager.handle_rename(&old_path, &new_path)` çalıştırılır:
     - Tam eşleşen veya `old_norm + "/"` önekiyle eşleşen mevcut `drive_files` satırlarının `local_path` ve `remote_path` sütunları güncellenir (böylece duraklatılmış modda bile veritabanındaki yol eşleşmesi bozulmaz); hedef yolda eski bir yetim satır varsa (üzerine taşıma) eski satır temizlenir.
     - Güncellenen satırlar `dirty = 1` işaretlenir; `drive_enabled == true` ise `idle_deadline` güncellenip arka plan işçisi uyandırılır (duraklatılmışsa sonraki `"Şimdi Yedekle"` veya yeniden etkinleştirme sırasında Drive'da taşınır).
3. `fs_ops::delete_file`:
   - **Drive'da silme kesinlikle çağrılmaz.** Kuyruktaki bir satır işlenirken dosya diskte `NotFound` verirse satır `dirty = 0` yapılır ve Drive'daki yedek olduğu gibi korunur.

**Klasör açma (`Open Folder...`) ve çalışma alanı tarama kapsamı:**
- **Klasör açma / oturum geri yükleme otomatik tarama yapmaz:** Kullanıcı `Open Folder...` ile bir klasör açtığında veya uygulama yeniden başlatıldığında klasördeki dokunulmamış tüm Markdown dosyaları otomatik taranıp yüklenmez (ör. içinde yüzlerce doküman bulunan bir kod deposu açıldığında istenmeyen toplu yükleme önlenir). Yalnızca OpenMD içinde kaydedilen/oluşturulan/adlandırılan/kurtarılan dosyalar ve önceki oturumdan kalan `dirty = 1` kuyruk kayıtları (açılıştan 2 sn sonra, `drive_enabled == true` ise) otomatik yedeklenir.
- **Bağlanma (`drive_connect`) ve manuel `"Şimdi Yedekle"` (`drive_sync_now`) tarama kapsamı:**
  - Her iki komut da `{ workspaceDir: Option<String>, openFilePaths: Vec<String> }` alır.
  - `workspaceDir` doluysa bu klasör `max_depth = 8` derinliğe kadar taranır (mevcut `scan_dir_recursive` hariç tutma kurallarıyla: gizli klasörler hariç `.cursor`/`.agents`, `node_modules`, `dist`, `dist-electron`, `target`) ve ek olarak çalışma alanı dışındaki açık sekmeler (`openFilePaths`) dahil edilir.
  - Açık çalışma alanı klasörü yoksa (`workspaceDir == None`), `~/Documents` gibi varsayılan klasörün tamamı **taranmaz**; yalnızca editörde o an açık ve diske kaydedilmiş sekmeler (`openFilePaths` içindeki geçerli `.md`/`.mdx`/`.markdown`, `<= 10 MiB` dosyalar) kontrol edilip kuyruğa eklenir.
  - Bulunan her uygun dosya için yerel SHA-256 hesaplanır; `drive_files` içinde kaydı olmayan veya `last_synced_hash != sha256` / `remote_path != last_synced_remote_path` olanlar `dirty = 1, next_retry_at = 0` olarak kuyruğa alınır. `drive_sync_now`, `drive_enabled == false` (duraklatılmış) olsa bile kullanıcı isteği üzerine çalışır ve tüm bekleyenleri hemen yükler.

**Durum öncelik sırası (`DriveSyncState`) ve arka plan zamanlayıcı:**
- `DriveManager` tek bir `tokio::spawn` arka plan döngüsü ve `sync_lock: tokio::sync::Mutex<()>` kullanır.
- Anlık `DriveStatus.state` şu kesin öncelik sırasıyla hesaplanır:
  1. `!connected` → `'disconnected'`
  2. Aktif yükleme sürüyor → `'syncing'`
  3. Token geçersiz / `invalid_grant` → `'auth_required'`
  4. `!settings.enabled` → `'paused'` *(Duraklatılmış modda manuel `drive_sync_now` hata verirse hata bilgisi komut dönüşünde ve `lastError` alanında iletilir; rozet `paused` kalır)*
  5. Son otomatik senkronizasyon ağ hatasıyla durduysa → `'offline'`
  6. Son otomatik senkronizasyon ağ dışı API/dosya hatasıyla durduysa → `'error'`
  7. `pending_count > 0` → `'pending'`
  8. Aksi halde → `'idle'`
- Arka plan zamanlayıcısı yalnızca `connected && drive_enabled && !auth_required` koşulunda tetiklenir ve iki koşuldan biri oluştuğunda bekleyen (`dirty = 1 AND next_retry_at <= now_ms`) dosyaları işler:
  1. **Hareketsizlik zamanlayıcısı:** Son `enqueue` sonrasında `drive_idle_delay_seconds` (varsayılan `30 sn`) boyunca yeni kayıt gelmediyse.
  2. **Periyodik zamanlayıcı:** Kullanıcı durmaksızın yazıp kaydetse bile son süpürmeden itibaren `drive_sync_interval_minutes` (varsayılan `5 dk`) dolduysa.
- **Uygulama açılışı:** Açılışta `drive_enabled == true` ve önceki oturumdan (`dirty = 1`) bekleyen dosya varsa `idle_delay` beklenmeden kısa başlangıç gecikmesiyle (`2 sn`) işlenmeye başlanır.
- **Tek dosya senkronizasyon adımı:**
  1. Yerel dosyayı oku: yoksa (`NotFound`) `dirty = 0` yap (Drive'da koru); boyutu `> 10 MiB` veya geçerli UTF-8 değilse `dirty = 0`, `last_error` yaz ve atla.
  2. `sha256(content)` hesapla. `last_synced_hash == Some(sha256) && drive_file_id.is_some() && last_synced_remote_path.as_deref() == Some(&remote_path)` ise ağ isteği yapmadan `dirty = 0` işaretle.
  3. Aksi halde `DriveClient` ile üst klasörü bul/oluştur ve yükle/taşı.
  4. Başarıda `drive_file_id`, `drive_parent_id`, `last_synced_hash`, `last_synced_remote_path`, `last_synced_at`, `dirty = 0`, `dirty_since = NULL`, `retry_count = 0`, `next_retry_at = 0`, `last_error = NULL` ve global `drive_last_sync_at` güncellenir.
- **Hata & Çevrimdışı Politikası:**
  - Bağlantı / DNS / timeout / HTTP 429 / 5xx hatalarında `dirty = 1` kalır, `retry_count += 1`, `next_retry_at = now_ms + backoff(retry_count)` (`30s → 60s → 120s → 300s → 900s`) uygulanır; durum `offline` (bağlantı hatası/timeout) veya `error` olur.
  - HTTP `401` / `invalid_grant` durumunda `auth_required = true` set edilir, arka plan yüklemeleri duraklatılır ve durum `auth_required` yayınlanır.

### 4.8 Uygulama Kapanışında Yedekleme (Quit Flush) & `Cmd+Q` Yakalama

Mevcut pencere kapatma akışının macOS `Cmd+Q` / menü Çıkışını da kapsaması için backend çıkışı bütüncül biçimde güncellenir:
- **`menu.rs` güncellemesi:** `PredefinedMenuItem::quit` macOS'ta doğrudan `[NSApplication terminate:]` gönderip pencere kapanış olayını atladığı için, bunun yerine `MenuItem::with_id(app, "quit-app", ..., true, Some("CmdOrCtrl+Q"))` kullanılır. `handle_menu_event` `"quit-app"` geldiğinde `main` pencereyi görünür/odaklı hale getirip `win.close()` çağırır; bu da hem `Cmd+Q` hem uygulama menüsü Çıkışını tek `WindowEvent::CloseRequested` → `app:window-closing` akışına yönlendirir.
- **`lib.rs` yaşam döngüsü:** `.run(...)` yerine `.build(...).run(|app, event| ...)` kullanılır:
  - `RunEvent::ExitRequested { api, .. }`: `AppState.force_close == false` ise `api.prevent_exit()` çağrılır, `main` pencere öne getirilip `app:window-closing` yayılır.
  - `close_window(window, state, app, force)`: `force == true` ise `state.force_close = true` ve `state.skip_exit_flush = true` set edilip `app.exit(0)` çağrılır (`cancel_close` ise ikisini de `false` yapar).
  - `RunEvent::Exit`: Dock menüsünden Doğrudan Çıkış gibi modal akışı atlayan acil macOS kapanışlarında `skip_exit_flush == false` ise `DriveManager` en fazla `3 saniye`lik bloklayıcı son şans flush dener; `skip_exit_flush == true` ise (`DriveQuitSyncModal` zaten tamamladı veya kullanıcı *"Beklemeden Kapat"* dedi) ek bekleme yapmaz.

**Frontend 2 aşamalı kapanış (`app:window-closing`):**
1. **Aşama A (Kaydedilmemiş Editör Sekmeleri):** `tabs.some(t => t.isModified)` doğruysa önce mevcut `UnsavedChangesModal` açılır.
   - *"Kaydet ve Kapat"* seçilirse tüm değiştirilmiş sekmeler diske yazılır (`drive_enabled == true` ise dosyalar hemen Drive kuyruğuna düşer) ve Aşama B'ye geçilir.
   - *"Kaydetmeden Kapat"* seçilirse yine Aşama B kontrol edilir; *"İptal"* seçilirse kapanış iptal edilir (`cancelClose()`).
2. **Aşama B (Bekleyen Google Drive Yedekleri):** `window.electronAPI.driveGetStatus()` kontrol edilir. Eğer `connected && settings.enabled && !authRequired && pendingCount > 0` ise `DriveQuitSyncModal` açılır ve `driveFlushBeforeQuit(10000)` çağrılır:
   - Modal ilerleme durumunu gösterir (`"Google Drive'a yedekleniyor… (N dosya kaldı)"`).
   - 10 saniye içinde bekleyen tüm dosyalar yüklenirse (`result.completed === true && result.remainingCount === 0`) modal kapanır ve `window.electronAPI.closeWindow(true)` uygulamayı kapatır.
   - Kullanıcı her an **"Beklemeden Kapat"** (`closeWindow(true)`) diyerek anında çıkabilir (`dirty = 1` kayıtlar SQLite'ta kalır, sonraki açılışta otomatik yüklenir).
   - 10 saniye dolduğunda veya ağ çevrimdışı hatasında modal kullanıcıya açık mesaj gösterir ve **"Tekrar Dene"** / **"Yine de Kapat"** / **"İptal"** (`cancelClose()`) düğmelerini sunar.

### 4.9 Tauri IPC Sözleşmesi (`src-tauri/src/commands/drive_ops.rs`)

Tüm DTO'lar `#[serde(rename_all = "camelCase")]` kullanır.

**Veri Modelleri:**
- `DriveSyncState`: `'disconnected' | 'paused' | 'idle' | 'syncing' | 'pending' | 'offline' | 'auth_required' | 'error'`
- `DriveSettings`: `{ enabled: boolean, syncIntervalMinutes: number, idleDelaySeconds: number }`
- `DriveAuthConfig`: `{ hasBuiltInClient: boolean, usingCustomClient: boolean, customClientId: string, hasCustomClientSecret: boolean }`
- `DriveStatus`:
  ```ts
  {
    state: DriveSyncState;
    connected: boolean;
    accountEmail: string | null;
    rootFolderId: string | null;
    settings: DriveSettings;
    pendingCount: number;
    syncedCount: number;
    lastSyncAt: number | null; // Unix ms
    lastError: string | null;
    authInProgress: boolean;
    authConfig: DriveAuthConfig;
  }
  ```
- `DriveRemoteFile`:
  ```ts
  {
    fileId: string;
    name: string;
    remotePath: string;     // Örn. 'Documents/Notlarim/plan.md'
    sizeBytes: number;
    modifiedTimeMs: number | null;
  }
  ```
- `DriveRestoreResult`: `{ filePath: string, fileName: string, content: string }`
- `DriveBatchRestoreResult`: `{ restoredCount: number, skippedCount: number, failedFiles: string[], targetDir: string }`
- `DriveFlushResult`: `{ completed: boolean, uploadedCount: number, remainingCount: number, timedOut: boolean, error: string | null }`

**Tauri Komutları (tümü `lib.rs` `invoke_handler!` + `tauriBridge.ts` içine eklenir):**

| Komut | Argümanlar | Dönüş | Açıklama |
|---|---|---|---|
| `drive_get_status` | `—` | `DriveStatus` | Anlık bağlantı/kuyruk durumu ve ayarları döner |
| `drive_update_settings` | `settings: DriveSettings` | `DriveStatus` | Aralıkları doğrular (`1..=60 dk`, `10..=300 sn`), kaydeder |
| `drive_save_custom_oauth` | `clientId: String, clientSecret: Option<String>` | `DriveAuthConfig` | BYO client ID'yi SQLite'a, opsiyonel secret'ı Keychain'e kaydeder; istemci değiştiyse mevcut oturumu temizler |
| `drive_clear_custom_oauth` | `—` | `DriveAuthConfig` | BYO ayarlarını temizler; istemci değiştiyse oturumu temizler |
| `drive_connect` | `workspaceDir: Option<String>, openFilePaths: Option<Vec<String>>` | `DriveStatus` | PKCE + loopback OAuth başlatır, `OpenMD/` kökünü çözümler, açık çalışma alanını ve/veya kaydedilmiş açık sekmeleri kuyruğa alıp arka planda yüklemeyi başlatır |
| `drive_cancel_auth` | `—` | `boolean` | Bekleyen OAuth loopback dinleyicisini iptal eder |
| `drive_disconnect` | `—` | `DriveStatus` | Token'ı revoke eder, Keychain + Drive SQLite önbelleğini temizler |
| `drive_sync_now` | `workspaceDir: Option<String>, openFilePaths: Option<Vec<String>>` | `DriveStatus` | Çalışma alanını ve/veya açık sekmeleri tarayıp değişenleri kuyruğa alır; `paused` durumda bile manuel olarak tüm bekleyenleri hemen yükler |
| `drive_flush_before_quit` | `timeoutMs: Option<u64>` | `DriveFlushResult` | Kapanış öncesi bekleyen yüklemeleri en fazla `timeoutMs` (varsayılan `10000`) içinde tamamlar |
| `drive_list_remote_files` | `—` | `DriveRemoteFile[]` | Drive'daki `OpenMD/` alt ağacını canlı listeler (sayfalı, max 5000 öğe) |
| `drive_download_file_content` | `fileId: String` | `String` | Seçili uzak dosyanın UTF-8 içeriğini (`<= 10 MiB`) belleğe indirir |
| `drive_restore_file` | `fileId: String, remotePath: String, targetDir: String, preserveFolders: bool` | `DriveRestoreResult` | İndirip yol bileşenlerini doğrular; `preserveFolders=true` ise alt klasörleri oluşturur, `resolve_available_path` ile çakışmasız diske yazar |
| `drive_restore_all` | `targetDir: String, preserveFolders: bool` | `DriveBatchRestoreResult` | Tüm uzak dosyaları seçilen klasöre çakışmasız indirir |

**Backend Event:**
- `drive:status-changed` (`DriveStatus` payload): OAuth durumu değiştiğinde, dosya kuyruğa girdiğinde, dosya yüklendiğinde veya ağ hatası oluştuğunda frontend'e yayınlanır (`tauriBridge.ts` `onDriveStatusChanged`).

---

## 5. Frontend Arayüzü ve Kullanıcı Deneyimi

*(Tarayıcı modunda `!isTauri()` ise aşağıdaki tüm Google Drive bileşenleri, durum çubuğu göstergesi, menü/komut girdileri ve Ayarlar sekmesi gizlenir.)*

### 5.1 Ayarlar > Bulut Yedekleme Sekmesi (`SettingsModal.tsx`)
- `SettingsModal`'a `initialTab?: 'general' | 'files' | 'editor' | 'cloud' | 'about'` prop'u eklenir (durum çubuğuna tıklanınca doğrudan `'cloud'` sekmesiyle açılır).
- 5. sekme: **`Bulut Yedekleme` / `Cloud Backup`** (`Cloud` ikonu).
- İçerik bölümleri:
  1. **Bağlantı Kartı:**
     - Bağlı değilken: *"Google Drive ile Bağlan"* (`driveConnect(rootFolderPath, openFilePaths)`) butonu, bağlanırken spinner + *"İptal Et"* butonu, `drive.file` kapsamının yalnızca OpenMD klasörüne eriştiğini açıklayan kısa güvence metni. (Eğer `hasBuiltInClient === false && !usingCustomClient` ise doğrudan gelişmiş BYO bölümünü açmaya yönlendiren rehber kutusu gösterilir.)
     - Bağlıyken / `paused` / `auth_required` durumunda: Bağlı Google e-posta adresi, durum rozeti, *"Şimdi Yedekle"* (`driveSyncNow(rootFolderPath, openFilePaths)` — duraklatılmış olsa bile aktiftir), *"Drive'dan Geri Yükle..."*, *"Yeniden Bağlan"* (`auth_required` ise) ve onaylı *"Bağlantıyı Kes"* butonu.
  2. **Otomatik Yedekleme Ayarları:**
     - Otomatik Bulut Yedeklemeyi Etkinleştir (toggle; kapatıldığında `paused` moda geçer)
     - Hareketsizlik Sonrası Bekleme (`15 sn`, `30 sn (Varsayılan)`, `60 sn`, `120 sn`)
     - Periyodik Yedekleme Aralığı (`1 dk`, `3 dk`, `5 dk (Varsayılan)`, `10 dk`, `15 dk`, `30 dk`)
  3. **Özet İstatistik Satırı:** Yedeklenen dosya sayısı (`syncedCount`), bekleyen dosya sayısı (`pendingCount`) ve son yedekleme zamanı (`lastSyncAt`, göreli formatla).
  4. **Gelişmiş: Özel OAuth İstemcisi (BYO) (daraltılabilir akordeon):**
     - `Client ID` metin alanı, `Client Secret` şifre alanı (kaydedilmişse placeholder `"Kayıtlı (değiştirmek için yeni değer girin)"` — eski secret asla gösterilmez), *"Kaydet"*, *"Varsayılana Dön"* ve GCP Masaüstü OAuth kurulumunu 3 adımda anlatan kısa yönerge.

### 5.2 Durum Çubuğu Göstergesi (`StatusBar.tsx`)
- Sağ kısımda `AutoSave` butonunun hemen soluna kompakt, tek satır, tıklanabilir bir bulut durumu düğmesi eklenir (tıklandığında Ayarlar > Bulut Yedekleme sekmesini açar):
  - `disconnected`: Sönük bulut ikonu + `Drive: Bağlı Değil` / `Drive: Off`
  - `paused`: Turuncu/gri duraklatma bulut ikonu + `Drive: Duraklatıldı` / `Drive: Paused`
  - `syncing`: Dönüyor/bulut ikonu + `Yedekleniyor (N)…`
  - `pending`: Sarı/amber nokta + `Bekliyor (N)`
  - `idle`: Yeşil bulut tik ikonu + `Drive Güncel`
  - `offline`: Gri/turuncu bulut-off ikonu + `Çevrimdışı (N bekliyor)`
  - `auth_required` / `error`: Kırmızı uyarı ikonu + `Drive: Yeniden Bağlan` / `Yedekleme Hatası`

### 5.3 Drive'dan Geri Yükleme Penceresi (`DriveRestoreModal.tsx`)
- Düzen olarak `HistoryModal` estetiğini izler:
  - Üst çubuk: Arama kutusu (dosya adı veya klasör yoluna göre filtreleme) + *"Yenile"* (`driveListRemoteFiles`) + *"Tümünü Klasöre İndir..."*.
  - Sol panel: Drive `OpenMD/` altındaki dosyaların listesi (dosya adı, uzak klasör yolu `Documents/Notlarim`, boyut, değiştirilme tarihi).
  - Sağ panel: Seçili dosyanın anlık indirilen salt okunur UTF-8 önizlemesi ve alt aksiyon çubuğu:
    - **"Yeni Sekmede Aç"** (diske yazmadan isimsiz düzenlenebilir sekme açar)
    - **"Klasöre Geri Yükle..."** (`openDirectoryDialog` ile klasör seçtirir, "Klasör yapısını koru" onay kutusu varsayılan işaretli; diske yazıldıktan sonra yeni sekmede açar ve açık çalışma alanı içindeyse ağaç görünümünü yeniler).

### 5.4 Kapanış Yedekleme Penceresi (`DriveQuitSyncModal.tsx`)
- Uygulama kapatılırken `pendingCount > 0` ise açılır.
- İçerik: İlerleme göstergesi, kalan dosya sayısı, 10 saniyelik bekleme durumu; **"Beklemeden Kapat"**, zaman aşımı/hatada **"Tekrar Dene"**, **"Yine de Kapat"** ve **"İptal"** düğmeleri.

### 5.5 Menü ve Komut Paleti
- `src-tauri/src/menu.rs` (`Dosya / File` menüsü, `Sürüm Geçmişi...` altına ayırıcı ile):
  - `drive-sync-now`: `"Google Drive'a Şimdi Yedekle"` / `"Back Up to Google Drive Now"`
  - `drive-restore`: `"Google Drive'dan Geri Yükle..."` / `"Restore from Google Drive..."`
- `CommandPalette` (`src/App.tsx`): Aynı iki komut + `"Bulut Yedekleme Ayarları"` komutu.

### 5.6 Çeviri ve Yardımcı Dosyalar
- `src/utils/driveUtils.ts`: Hata kodlarını kullanıcı dostu TR/EN anahtarlara eşleme, bekleme/durum etiketi yardımcıları, uzak dosya listesi filtreleme yardımcıları (Node test runner ile bağımsız birim test edilebilir).
- `src/i18n/translations.ts`: Tüm `drive.*` ve `settings.cloud*` / `status.drive*` anahtarları `tr` ve `en` sözlüklerinde birebir eşleşen sayıyla eklenir.

---

## 6. Güvenlik ve Hata Dayanıklılığı

- **Yol Enjeksiyonu & Traversal Koruması:** `drive_restore_file` ve `drive_restore_all`, uzak yolun her segmentini `fs_ops::validate_single_component` ile doğrular (`.` / `..` / `/` / `\` / NUL içeren bozuk uzak adlar reddedilir) ve nihai hedef dosya adını `resolve_available_path` ile çakışmasız belirler.
- **Drive API Sorgu Enjeksiyonu Koruması:** Klasör ve dosya aramalarında tek tırnak `'` ve ters eğik çizgi `\` karakterleri kaçılır.
- **Dosya Boyut Sınırı (`10 MiB`):** Hem yükleme hem indirme `10 * 1024 * 1024` bayt ile sınırlandırılır.
- **OAuth Güvenliği:** Loopback yalnızca `127.0.0.1` üzerinde dinler, tek kullanımlık 256-bit `state` zorunludur ve S256 PKCE kullanılır.
- **Eşzamanlılık Güvenliği:**
  - Keychain token yenileme tek bir `auth_lock` arkasında serileştirilir; aynı anda birden fazla `invalid_grant` yarışı oluşmaz.
  - Drive senkronizasyon adımları tek bir `sync_lock` altında çalışır; bu nedenle aynı anda iki worker aynı klasörü Drive'da çift oluşturmaz.
  - `flush_before_quit` beklerken `DriveManager` arka plan döngüsü aynı `sync_lock`'u paylaştığı için mükerrer yükleme oluşmaz.

---

## 7. Test ve Doğrulama Stratejisi

### Rust Testleri (`cargo test --manifest-path src-tauri/Cargo.toml`)
`drive_auth.rs`, `drive_client.rs`, `drive_sync.rs`, `history_store.rs` ve `commands/drive_ops.rs` içinde `tempfile::tempdir()`, `MemoryTokenStore` ve yerel `tokio::net::TcpListener` tabanlı sahte Google OAuth/Drive HTTP sunucusu kullanılarak şu testler yazılır:
1. **DB Migrasyonu (`v1 → v2`):** Mevcut v1 `history.db` (içinde ayar + snapshot varken) açıldığında `user_version = 2`'ye yükseldiğini, v1 verilerinin korunduğunu ve yeni `drive_files` / `drive_folders` / `drive_*` ayarlarının oluştuğunu doğrular.
2. **Yol Eşleme & Filtreleme:** `$HOME` içi (`Documents/Notlar/a.md`), `$HOME` dışı (`_external/...`), `.` / `..` traversal reddi, yalnızca `.md`/`.mdx`/`.markdown` kabulü.
3. **OAuth PKCE & Loopback:** `state` uyuşmazlığı reddi, `access_denied` reddi, `drive_cancel_auth` ile iptal, başarılı kod → token + email değişimi (BYO `client_secret` dahil ve hariç), tekil token yenileme ve `invalid_grant` → `auth_required` geçişi, `disconnect` temizliği.
4. **Drive İstemcisi:** Tek tırnaklı klasör adlarında `q` kaçışı, klasör önbellekleme, `404` kendi kendini onarma (kök ve alt klasör yeniden kurma), aynı isimle mevcut dosya bulup üzerine PATCH, multipart oluşturma, media PATCH, `addParents`/`removeParents` taşıma/yeniden adlandırma, sayfalı `nextPageToken` listeleme, 10 MiB indirme sınırı.
5. **Senkronizasyon Motoru:** `SHA-256` + `last_synced_remote_path` değişmemişse sıfır ağ isteği, `rename_file` sonrası içerik değişmese bile Drive taşıması, yerel silmede Drive'dan silmeme, ağ hatasında üstel backoff (`next_retry_at > now`) ve tekrar başarıda temizleme, `flush_before_quit` başarı ve timeout davranışı.
6. **Geri Yükleme Komutları:** `drive_restore_file` klasör koruma + çakışma anında `"Not 2.md"` üretimi + traversal karakteri içeren uzak yol segmenti reddi.

### Frontend & Genel Doğrulama
- `test/test-utils.js`: `driveUtils` birim testleri + `tr`/`en` tüm anahtarların eşitlik kontrolü (Node `--experimental-strip-types`).
- Tam doğrulama kapısı:
  - `npm run typecheck` (0 hata)
  - `npm test` (tüm bölümler geçer)
  - `npm run build` (hatasız üretim derlemesi)
  - `cargo test --manifest-path src-tauri/Cargo.toml` (tüm testler geçer)
  - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` (0 uyarı)

---

## 8. Uygulayıcıya (Builder) Bırakılanlar

- `src-tauri/Cargo.toml` içinde Rust edition 2024 / MSRV 1.90 uyumlu exact minor/patch crate sürümlerinin seçimi (`reqwest` 0.12, `keyring` 3, `rand` 0.9) ve macOS üzerinde RFC 3339 `modifiedTime` tarih dizesini Unix milisaniyesine çeviren küçük yardımcı fonksiyon (harici ağır tarih crate'i olmadan veya mevcut std/basit parser ile).
- `DriveRestoreModal`, `DriveQuitSyncModal`, `SettingsModal` Bulut sekmesi ve `StatusBar` rozetinin `src/index.css` koyu/açık tema değişkenlerine uyumlu CSS detayları ve animasyonları.
- İsteğe bağlı hata detay metinlerinin ve buton etiketlerinin birebir Türkçe/İngilizce ifade cilası.
