export const DEFAULT_WELCOME_MARKDOWN = `# Hoş Geldiniz! 🚀 OpenMD Markdown Editörü

**OpenMD**, macOS için tasarlanmış modern, hızlı ve estetik bir açık kaynak Markdown editörüdür. Kod yazarken, not tutarken veya dokümantasyon hazırlarken en akıcı deneyimi sunar.

> [!NOTE]
> OpenMD, macOS Human Interface Guidelines (HIG) ilkelerine göre tasarlanmıştır. \`Cmd+K\` tuşlarına basarak **Komut Paleti**'ne dilediğiniz an erişebilirsiniz.

---

## ⚡ Hızlı Kısayollar

| Özellik | macOS Kısayolu | Açıklama |
| :--- | :--- | :--- |
| **Yeni Belge** | \`Cmd + N\` | Yeni boş bir sekme açar |
| **Dosya Aç** | \`Cmd + O\` | Bilgisayarınızdan bir \`.md\` dosyası seçer |
| **Klasör Aç** | \`Cmd + Shift + O\` | Çalışma alanı klasör ağacını açar |
| **Kaydet** | \`Cmd + S\` | Mevcut belgeyi diske kaydeder |
| **Kenar Çubuğu** | \`Cmd + \\\` | Dosya gezgini ve başlık ağacını açar/kapatır |
| **Görünüm Modları** | \`Cmd + 1 / 2 / 3\` | Sadece Editör / Bölünmüş / Önizleme |
| **Komut Paleti** | \`Cmd + K\` | Hızlı eylem arama penceresi |
| **Dışa Aktar** | \`HTML / PDF\` | Tek tıkla stilize dışa aktarma |

---

## 📋 İnteraktif Görev Listesi

Aşağıdaki görevleri doğrudan önizleme ekranından işaretleyebilirsiniz:
- [x] macOS Pencere Entegrasyonu ve Cam Efekti
- [x] CodeMirror 6 Sözdizimi Vurgulama
- [x] Canlı Bölünmüş Ekran (Split-view)
- [ ] Kendi Markdown dosyanızı açın (\`Cmd+O\`)
- [ ] Belgeyi PDF olarak dışa aktarmayı deneyin

---

## 🧮 Matematiksel Formüller (KaTeX Desteği)

İster satır içi formüller $f(x) = \\int_{-\\infty}^\\infty \\hat{f}(\\xi)\\,e^{2 \\pi i \\xi x}\\,d\\xi$, ister blok formüller oluşturun:

$$
E = mc^2 \\quad \\Longleftrightarrow \\quad \\nabla \\times \\mathbf{B} = \\mu_0\\mathbf{J} + \\mu_0\\epsilon_0\\frac{\\partial\\mathbf{E}}{\\partial t}
$$

---

## 📊 Akıllı Diyagramlar (Mermaid.js)

Kod blokları içinde doğrudan akış şemaları ve mimari diyagramlar çizebilirsiniz:

\`\`\`mermaid
graph TD
    A[Markdown Metni] -->|CodeMirror 6| B(Anlık Parser)
    B --> C{Render Modu}
    C -->|Split View| D[GFM + KaTeX + Mermaid]
    C -->|Export| E[Stilli Bağımsız HTML & PDF]
    D --> F[macOS Görsel Şölen]
\`\`\`

---

## 💻 Kod Blokları ve Sözdizimi Renklendirme

\`\`\`typescript
interface DocumentStats {
  words: number;
  characters: number;
  readingTimeMinutes: number;
}

export function calculateStats(content: string): DocumentStats {
  const words = content.trim().split(/\\s+/).filter(Boolean).length;
  const characters = content.length;
  const readingTimeMinutes = Math.ceil(words / 200);
  return { words, characters, readingTimeMinutes };
}
\`\`\`

---

> [!TIP]
> Sol kenar çubuğundaki **Anahat (Outline)** sekmesine tıklayarak belgenizdeki başlıklara anında zıplayabilirsiniz!

Keyifli yazmalar! ✍️
`;
