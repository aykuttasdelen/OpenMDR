# Yerel Sürüm Geçmişi ve Dosya Kurtarma (Aşama 1) — Tasarım Dokümanı

**Tarih:** 2026-11-21
**Proje:** OpenMD (`openmdR` — Rust / Tauri v2 + React 19 + TypeScript + CodeMirror 6)
**Kapsam:** Aşama 1 (Yerel sürüm geçmişi, satır bazlı fark görünümü, geri yükleme ve silinen dosya kurtarma). Google Drive bulut yedekleme ayrı bir proje olarak Aşama 2'ye bırakılmıştır.

---

## 1. Amaç ve Neden

OpenMD'de kullanıcılar Markdown/MDX belgeleri üzerinde çalışırken:
1. Düzenleme sırasında yanlışlıkla içeriği bozduğunda veya üzerine yazdığında dosyanın önceki sürümlerine kolayca geri dönebilmeli.
2. Bir dosyayı ya da tüm çalışma klasörünü yanlışlıkla sildiğinde, OpenMD'de daha önce kaydettiği sürümlere çalışma klasöründen bağımsız güvenli bir konumdan ulaşabilip dosyayı kurtarabilmeli.

Ayrı bir "10 dakika geriden gelen klasör kopyalayıcısı" yerine, sürüm geçmişini kullanıcının çalışma klasörünün dışındaki korumalı uygulama veri dizininde tutarak hem sürüm geri alma hem de silinmeye karşı kurtarma ihtiyacını tek, atomik ve düşük disk yüklü bir sistemle çözüyoruz.

---

## 2. Hedefler, Hedef Dışındakiler ve Anti-Hedefler

### Hedefler
- **Dosya başına ayarlanabilir sürüm limiti:** Ayarlar > Dosyalar sekmesi altında dosya başına tutulacak maksimum sürüm sayısı ayarlanabilmeli (varsayılan `50`, aralık `5–200`).
- **Hibrit sürüm alma politikası:**
  - Manuel kayıt (`Cmd+S`, Farklı Kaydet, Kapanışta Tümünü Kaydet): içerik son sürümden farklıysa anında sürüm alınır.
  - Otomatik kayıt (`auto-save`, 1 sn debounce): dosya diske normal şekilde yazılır, sürüm geçmişine ise yalnızca içerik değişmiş **ve** o dosyanın son sürümünden bu yana belirlenen süre (varsayılan `3 dakika`, ayarlardan değiştirilebilir) geçmişse eklenir.
  - **İlk düzenleme koruması:** Diskte zaten var olan ve geçmişte henüz kaydı bulunmayan bir dosya OpenMD'de ilk kez değiştirilip kaydedildiğinde, üzerine yazmadan hemen önce diskteki eski içerik `initial` sürümü olarak kaydedilir.
  - **Geri alma güvenliği:** Eski bir sürüme geri dönmeden önce editördeki güncel içerik (kaydedilmemiş değişiklikler dahil) `pre_restore` sürümü olarak kaydedilir.
- **İçerik tekilleştirme (deduplication):** Yeni içeriğin SHA-256 özeti dosyanın son sürümüyle aynıysa yeni kayıt oluşturulmaz.
- **Yeniden adlandırma takibi:** OpenMD içinden dosya veya klasör yeniden adlandırıldığında (`fs_ops::rename_file`) ilgili sürüm kayıtları yeni yola taşınır.
- **Silinen / kayıp dosyaları kurtarma:** Diskte artık bulunmayan ama veritabanında sürümü olan dosyalar ayrı bir sekmede listelenir; kullanıcı bu dosyaları orijinal konumuna (gerekirse üst klasörleri yeniden oluşturarak) veya seçeceği yeni bir klasöre kurtarabilir.
- **Satır bazlı Diff ve İçerik Önizleme:** Geçmiş penceresinde seçili sürüm ile güncel dosya içeriği arasındaki eklenen/silinen satırlar renkli fark (diff) olarak görüntülenir; tek tıkla tam içerik görünümüne geçilebilir.
- **İki geri yükleme modu:** "Bu sürüme geri dön" (dosyayı ve açık sekmeyi güncelle) ve "Yeni sekmede kopya aç" (orijinale dokunmadan yeni sekmede aç).
- **Depo yönetimi ve şeffaflık:** Ayarlar ekranında sürüm geçmişini açma/kapatma, toplam depo boyutu / dosya / sürüm istatistiği ve geçmiş temizleme (tek dosya veya tüm geçmiş) seçenekleri.
- **İki dilli arayüz:** Mevcut `src/i18n/translations.ts` yapısına uygun Türkçe ve İngilizce destek.

### Hedef Dışındakiler (Aşama 1)
- Google Drive / bulut yedekleme ve OAuth akışı (Aşama 2).
- İkili (binary) dosyaların ve `assets/` altındaki görsellerin yedeklenmesi (Aşama 1 yalnızca OpenMD üzerinden yazılan metin/Markdown belgelerini kapsar).
- OpenMD dışındaki uygulamaların yaptığı değişiklikleri izleyen arka plan dosya sistemi izleyici (file watcher).
- 3 yollu birleştirme (3-way merge) veya karakter içi ince diff.

### Anti-Hedefler
- **Dosya kaydını bozma/engelleme:** SQLite hatası veya I/O gecikmesi hiçbir koşulda kullanıcının asıl dosya yazımını (`write_file` / `save_file_dialog`) başarısız kılamaz; geçmiş kayıt hataları loglanır ama asıl kayıt devam eder.
- **Auto-save yüzünden geçmişin şişmesi:** 1 saniyelik otomatik kayıt her vuruşta yeni sürüm üretip 50 kaydı birkaç dakikada tüketmemelidir.
- **Sessiz veri kaybı:** Eski sürüme dönerken kullanıcının editördeki kaydedilmemiş değişiklikleri `pre_restore` olarak saklanmadan silinmemelidir.
- **Arayüz donması:** Büyük belgelerde diff hesabı veya SQLite işlemleri ana UI thread'ini / async runtime'ı bloklamamalıdır.
- **Güvensiz dosya yolu yazımı:** Kurtarma işlemleri path traversal içermemeli ve farklı klasöre kurtarmada mevcut dosyaların üzerine sessizce yazmamalıdır.

---

## 3. Değerlendirilen Yaklaşımlar ve Seçim

### Baştan Elenenler
- **Çalışma klasörü içinde gizli `.history/` klasörü:** Kullanıcı çalışma klasörünü sildiğinde yedekler de yok olur ve kullanıcı klasör içeriğine müdahale edebilir.
- **Gömülü yerel Git deposu (`git2` / `git` CLI):** Dosya başına tam `N` sürüm tutup fazlasını budama gereksinimi Git'in repo-geneli commit grafiğiyle doğrudan çatışır (eski tek-dosya budaması tarihçe yeniden yazımı gerektirir).

### Karşılaştırılan Yaklaşımlar

| Kriter | Yaklaşım A: Tek SQLite Veritabanı (`rusqlite`, bundled) *(Seçilen)* | Yaklaşım B: Dosya Sistemi (JSON İndeks + Snapshot Dosyaları) |
|---|---|---|
| **Veri bütünlüğü** | WAL modu + atomik transaction; çökmede yarım/bozuk kayıt yok | Eşzamanlı auto-save/rename/budamada özel mutex + atomic temp-rename gerekir |
| **İzolasyon** | Tek binary `history.db`; Finder'da yanlışlıkla parça parça kurcalanmaz | Çok sayıda düz dosya ve klasör |
| **Performans & Disk** | Dosya başına budama, rename, boyut ve silinen dosya sorgusu tek SQL ile hızlı; düşük disk yorgunluğu | Çok dosya/klasörde tarama ve JSON yeniden yazımı disk I/O yaratır |
| **Aşama 2 (Drive) uyumu** | Aynı DB'ye `synced_at` / `drive_file_id` sütunu eklemek kolay | JSON şeması karmaşıklaşır |
| **Maliyet** | `Cargo.toml`'a `rusqlite` (`bundled`), `flate2`, `sha2`, `similar` eklenir (~1-1.5 MB binary artışı) | Sıfır DB bağımlılığı |

**Seçilen Yaklaşım:** **Yaklaşım A (SQLite — `rusqlite` bundled)**. Kullanıcı tarafından disk yorgunluğunu önleme, performans ve bütünlük gerekçeleriyle onaylanmıştır.

---

## 4. Mimari ve Teknik Tasarım

### 4.1 Depo Konumu ve Yaşam Döngüsü
- **Konum:** `app.path().app_local_data_dir()?.join("history.db")` (macOS'ta `~/Library/Application Support/com.openmd.editor/history.db`). Üst klasör yoksa oluşturulur.
- **Yönetim (`AppState`):** `src-tauri/src/lib.rs` içindeki `AppState` yapısına `HistoryManager` (içinde `Arc<Mutex<Connection>>` veya DB path + güvenli bağlantı yönetimi) eklenir. *(Kararımız: Tüm SQLite erişimi `tokio::task::spawn_blocking` içinde çalıştırılır; `PRAGMA journal_mode = WAL;`, `PRAGMA foreign_keys = ON;`, `PRAGMA busy_timeout = 5000;` ve şema versiyonlaması için `PRAGMA user_version = 1;` açılışta uygulanır.)*
- **Ayarların kaynağı:** *(Kararımız)* Sürüm geçmişi ayarları (`enabled`, `max_versions_per_file`, `autosave_interval_minutes`) doğrudan `history.db` içindeki `settings` tablosunda kalıcı olarak saklanır ve uygulama açılışında frontend `history_get_settings` komutuyla okur. Böylece uygulama açılır açılmaz yapılan bir kayıtta `localStorage` senkronizasyon yarış durumu (race condition) yaşanmaz.

### 4.2 Veritabanı Şeması (`PRAGMA user_version = 1`)

```sql
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
```

- **Varsayılan ayar değerleri:**
  - `enabled`: `true`
  - `max_versions_per_file`: `50` (geçerli aralık `5..=200`)
  - `autosave_interval_minutes`: `3` (geçerli aralık `1..=60`)
- **Boyut sınırı:** *(Kararımız)* Tekil snapshot üst boyutu `10 MB` (uncompressed UTF-8). Bu boyutu aşan dosyalarda snapshot sessizce atlanır ve uyarı loglanır; böylece yanlışlıkla açılan devasa bir metin/log dosyası DB'yi şişirmez.
- **Yol normalizasyonu:** *(Kararımız)* `file_path` kaydedilirken sondaki `/` temizlenir ve mümkünse mevcut üst dizin üzerinden tutarlı bir mantıksal mutlak yol kullanılır (symlink çözümlemesi yüzünden OpenMD'nin `Tab.filePath` değeriyle uyuşmazlık olmaması için OpenMD'nin kullandığı mutlak yol formatı korunur).

### 4.3 Kayıt ve Tetikleme Kuralları

Mevcut yazma noktalarının tamamı entegre edilir:

1. **`fs_ops::write_file(app, state, file_path, content, save_source: Option<String>)`:**
   - `save_source` değerleri: `"manual"` (varsayılan, geriye uyumlu), `"autosave"`, `"initial"`.
   - `App.tsx` içindeki auto-save `useEffect`'i `"autosave"`, manuel `handleSaveFile` / `handleSaveAllAndClose` `"manual"`, `handleNewFile` / `handleOCRComplete` `"initial"` gönderir.
   - **Yazma öncesi (`initial` taban yakalama):** `enabled == true` ise ve `file_path` diskte zaten varsa **ve** `history.db`'de bu dosya için henüz hiç snapshot yoksa, diskteki mevcut dosya (<= 10 MB, geçerli UTF-8 ise) okunur ve yeni içerikten farklıysa `trigger = 'initial'` olarak kaydedilir.
   - **Asıl dosya yazımı:** `tokio::fs::write(&file_path, &content)` her zaman normal şekilde çalışır.
   - **Yazma sonrası snapshot:** `enabled == true` ise:
     - `content_hash = sha256(content)`.
     - Son snapshot varsa ve `latest.content_hash == content_hash` ise atlanır.
     - `save_source == "autosave"` ve son snapshot varsa ve `(now_ms - latest.created_at) < autosave_interval_minutes * 60_000` ise atlanır.
     - Aksi halde yeni snapshot `gzip` (`flate2`) ile sıkıştırılıp eklenir.
     - Dosya için `snapshots` sayısı `max_versions_per_file` değerini aşıyorsa en eski fazla kayıtlar aynı transaction içinde silinir.
2. **`dialogs::save_file_dialog(...)`:**
   - Kullanıcı bir dosya yolu seçip içerik diske yazıldığında aynı yazma-öncesi taban yakalama + `'manual'` snapshot akışı uygulanır.
3. **`fs_ops::rename_file(state, old_path, new_path)`:**
   - `tokio::fs::rename` başarılı olduktan sonra:
     - Tam eşleşen `file_path = old_path` kaydı varsa `new_path` ve yeni `file_name` ile güncellenir. (Eğer `new_path` daha önce silinmiş bir kayıt olarak `tracked_files` içinde duruyorsa, eski kaydın snapshot'ları çakışmaması için yeni dosyaya birleştirilir veya eski yetim kayıt temizlenir — *(Kararımız: hedef yolda mevcut eski kayıt varsa snapshot'ları korunup yeni yola taşınacak kaydın altına birleştirilir, ardından eski `tracked_files` satırı silinip `file_path` güncellenir, sonra limit budaması yapılır.)*)
     - Eğer yeniden adlandırılan şey bir **klasörse**, `file_path` değeri `old_path + "/"` (veya platform ayırıcısı) ile başlayan tüm `tracked_files` satırlarının ön eki `new_path + "/"` ile güncellenir.
4. **`fs_ops::delete_file(file_path)`:**
   - Mevcut davranış korunur (OS Çöp Kutusu'na taşır); `history.db` içindeki kayıtlar **silinmez**. Dosya diskte artık var olmadığı için "Silinen / Kayıp Dosyalar" sekmesinde görünür.
5. **Ayarlar güncellendiğinde (`history_update_settings`):**
   - *(Kararımız)* `max_versions_per_file` düşürülürse (ör. 50'den 20'ye), tüm takip edilen dosyalardaki limit üstü eski snapshot'lar anında budanır.

### 4.4 Geri Yükleme, Kopya Açma ve Silinen Dosya Kurtarma

1. **Mevcut dosyada eski sürüme geri dön (`history_restore_snapshot`):**
   - Girdi: `file_path`, `snapshot_id`, `current_content` (editördeki anlık metin; kaydedilmemiş değişiklikler dahil).
   - Adımlar:
     1. Önce `current_content` içeriği `trigger = 'pre_restore'` olarak kaydedilir (son snapshot ile hash aynıysa tekrar eklenmez).
     2. Hedef `snapshot_id` veritabanından okunup açılır (decompress) ve `file_path` konumuna diske yazılır.
     3. Geri yüklenen metin frontend'e döndürülür.
     4. Frontend ilgili açık sekmede `content` ve `savedContent` değerlerini günceller, `isModified = false` yapar. (`Editor.tsx` dışarıdan gelen `value` değişikliğini `isUpdatingFromProp` ile CodeMirror'a sorunsuz yansıttığı için editör anında güncellenir.)
2. **Yeni sekmede kopya aç:**
   - `history_get_snapshot_content(snapshot_id)` ile metin alınır.
   - Orijinal dosyaya dokunulmadan yeni bir isimsiz sekme açılır (başlık örn. `Belge (14-32 kopya).md`, `filePath: undefined`, `isModified: true`).
3. **Silinen / kayıp dosyaları listeleme (`history_list_deleted_files`):**
   - `tracked_files` tablosundaki en az 1 snapshot'a sahip tüm kayıtlar çekilir; `Path::new(&file_path).exists()` sonucu `false` olanlar döndürülür (en son güncellenene göre azalan sırada, snapshot sayısı ve son sürüm zamanı/boyutuyla birlikte).
4. **Silinen dosyayı kurtarma (`history_recover_file`):**
   - Girdi: `file_id`, `snapshot_id`, `target_dir: Option<String>`.
   - **`target_dir == None` (Orijinal konumuna kurtar):**
     - `tracked_files.file_path` hedef alınır. Üst klasör yoksa `tokio::fs::create_dir_all` ile oluşturulur.
     - Hedef yolda bu sırada başka bir dosya oluşmuşsa üzerine yazmak yerine `fs_ops::get_available_path` ile çakışmasız yeni isim üretilir.
   - **`target_dir == Some(dir)` (Farklı klasöre kurtar):**
     - `dir` + `tracked_files.file_name` için `fs_ops::get_available_path` ile çakışmasız yol üretilir (`validate_single_component` korunur).
   - İçerik diske yazılır; dosya yeni bir yola kurtarıldıysa `tracked_files.file_path` ve `file_name` güncellenir; kurtarılan yol ve içerik frontend'e döndürülür.
   - Frontend kurtarılan dosyayı hemen yeni sekmede açar ve açık çalışma alanı içindeyse dosya ağacını (`handleRefreshTree`) yeniler.
5. **Silinen dosya geçmişini kalıcı kaldırma (`history_delete_file_history`):**
   - Kullanıcı silinen bir dosyanın geçmişini artık istemiyorsa onay alarak `tracked_files` satırını (CASCADE ile snapshot'larını) silebilir.

### 4.5 Diff Motoru
- Rust backend'de `similar` crate'i (`TextDiff::from_lines(&snapshot_content, &current_content)`) kullanılır.
- `history_diff_snapshot(snapshot_id, current_content)` çıktısı:
  - `lines: Vec<DiffLine>` (`tag: "equal" | "insert" | "delete"`, `old_line_no: Option<usize>`, `new_line_no: Option<usize>`, `content: String`)
  - `added_count: usize`, `removed_count: usize`, `unchanged_count: usize`, `truncated: bool`
- *(Kararımız)* 50.000 satır üzerindeki dosyalarda diff satır çıktısı güvenli üst sınıra kırpılıp `truncated: true` işaretlenir ve arayüzde kullanıcıya tam içerik önizleme önerilir.
- Diff yönü: Seçili eski sürüm → güncel içerik (eski sürümde olup güncel dosyada silinmiş satırlar kırmızı `-`, sonradan eklenmiş satırlar yeşil `+` görünür; veya kullanıcı bunu net anlasın diye başlıkta `"Seçili sürüm → Güncel belge"` etiketi gösterilir).

### 4.6 Tauri IPC Komutları (`src-tauri/src/commands/history_ops.rs`)

| Komut | Amaç |
|---|---|
| `history_get_settings()` | Mevcut `HistorySettings` değerlerini döner |
| `history_update_settings(settings)` | Ayarları doğrular, kaydeder ve limit düştüyse fazla sürümleri budar |
| `history_list_snapshots(file_path)` | Dosyanın sürüm özet listesini döner (metin içeriği olmadan, hızlı) |
| `history_get_snapshot_content(snapshot_id)` | Tek bir sürümün tam metnini açıp döner |
| `history_diff_snapshot(snapshot_id, current_content)` | Seçili sürüm ile güncel içerik arasındaki satır diff'ini döner |
| `history_restore_snapshot(file_path, snapshot_id, current_content)` | `pre_restore` alır, diske yazar, geri yüklenen içeriği döner |
| `history_list_deleted_files()` | Diskte artık bulunmayan takip edilen dosyaları listeler |
| `history_list_snapshots_by_file_id(file_id)` | Silinen/kayıp bir dosyanın tüm sürümlerini `file_id` ile listeler |
| `history_recover_file(file_id, snapshot_id, target_dir)` | Silinen dosyayı orijinal yerine veya seçilen klasöre kurtarır |
| `history_delete_file_history(file_id)` | Tek dosyanın geçmişini kalıcı siler |
| `history_clear_all()` | Tüm geçmişi temizler ve `VACUUM` ile disk alanını iade eder |
| `history_get_stats()` | `{ totalFiles, totalSnapshots, dbSizeBytes }` döner |

Tüm komutlar `src-tauri/src/lib.rs` `invoke_handler!` içine, `src/api/tauriBridge.ts` ve `src/types/electron.d.ts` içine eklenir.

### 4.7 Arayüz Tasarımı ve Erişim Noktaları

**Erişim noktaları:**
- `TitleBar.tsx`: Saat/geçmiş ikonlu (`History`) buton → `HistoryModal` açar.
- `menu.rs` (`Dosya / File` menüsü): "Sürüm Geçmişi..." / "Version History..." (`CmdOrCtrl+Shift+H`).
- `CommandPalette.tsx`: "Sürüm Geçmişi" ve "Silinen Dosyaları Kurtar" komutları.
- `SettingsModal.tsx` (`Dosyalar` sekmesi):
  - Sürüm Geçmişini Etkinleştir (toggle)
  - Dosya Başına Maksimum Sürüm (sayı girişi / hızlı seçim, `5–200`, varsayılan `50`)
  - Otomatik Kayıt Sürüm Aralığı (`1, 3, 5, 10, 15, 30 dk`, varsayılan `3 dk`)
  - Depo bilgisi (toplam dosya, sürüm, KB/MB boyutu) + "Tüm Geçmişi Temizle" (onaylı)

**`HistoryModal.tsx` Düzeni (Metin Eskizi):**

```text
+-----------------------------------------------------------------------------+
| [Saat] Sürüm Geçmişi  —  notlarim.md                                    [X] |
+-----------------------------------------------------------------------------+
| [ Bu Dosya (14) ]   [ Silinen / Kayıp Dosyalar (2) ]                        |
+------------------------+----------------------------------------------------+
| SÜRÜMLER               | [ Fark (Diff) ] [ İçerik ]    +12 / -4 satır       |
|------------------------|----------------------------------------------------|
| ● Bugün 14:15          |  12   ## Kurulum                                   |
|   [Manuel] 42 satır    |  13 - Eski komut buraya yazılmıştı                 |
|                        |  13 + Yeni Tauri v2 komutu                         |
| ○ Bugün 14:08          |  14   Adımları sırayla izleyin.                    |
|   [Otomatik] 39 satır  |                                                    |
|                        |                                                    |
| ○ Bugün 13:50          |                                                    |
|   [Başlangıç] 35 satır |                                                    |
|                        |----------------------------------------------------|
|                        | [ Yeni Sekmede Kopya Aç ]   [ Bu Sürüme Geri Dön ] |
+------------------------+----------------------------------------------------+
```

- Aktif sekmenin henüz `filePath` değeri yoksa (ör. "Hoş Geldiniz.md") "Bu Dosya" sekmesinde bilgilendirici boş durum gösterilir; "Silinen / Kayıp Dosyalar" sekmesi her zaman erişilebilir kalır.
- "Silinen / Kayıp Dosyalar" sekmesinde sol listede silinen dosyalar (dosya adı, eski yol, son kayıt tarihi, sürüm sayısı); seçilince sağda sürümleri seçip tam içeriğini inceleme, **"Orijinal Konumuna Kurtar"**, **"Klasör Seçerek Kurtar..."** ve **"Geçmişten Kaldır"** butonları bulunur.

---

## 5. Uygulayıcıya (Builder) Bırakılanlar

- `Cargo.toml`'da Rust edition 2024 / MSRV 1.90 uyumlu güncel minor crate sürümlerinin seçimi (`rusqlite` with `["bundled"]`, `flate2`, `sha2`, `similar`).
- `src/index.css` içinde mevcut koyu/açık tema değişkenlerine uyumlu diff satır renkleri (`--diff-add-bg`, `--diff-del-bg`, `--diff-add-text`, `--diff-del-text` vb.) ve `HistoryModal` responsive stilleri.
- Göreli tarih formatlama yardımcısı (ör. "Az önce", "5 dk önce", "Dün 14:20" — TR/EN i18n uyumlu).
- Rust birim/entegrasyon testlerinin (`tempfile::tempdir()` ile izole `history.db` üzerinde: hibrit throttling, SHA-256 tekilleştirme, initial taban yakalama, limit budama, pre_restore + restore, dosya/klasör rename, silinen dosya tespiti ve kurtarma, bozuk/büyük dosya dayanıklılığı) ve mevcut `npm test` / `npm run typecheck` / `cargo test` doğrulamalarının yazılması.
