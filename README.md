# OpenMD 🚀

<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="OpenMD Logo" width="120" height="120" />
</p>

<p align="center">
  <strong>Modern, Ultra-Fast & Resource-Efficient Markdown & MDX Editor for macOS</strong>
  <br />
  <em>Engineered with Rust (Tauri v2) & React 19 + TypeScript</em>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Rust-1.90%2B-orange.svg?style=flat-square&logo=rust" alt="Rust" />
  <img src="https://img.shields.io/badge/Tauri-v2.12-blue.svg?style=flat-square&logo=tauri" alt="Tauri v2" />
  <img src="https://img.shields.io/badge/React-19-61dafb.svg?style=flat-square&logo=react" alt="React 19" />
  <img src="https://img.shields.io/badge/TypeScript-5.8-3178c6.svg?style=flat-square&logo=typescript" alt="TypeScript" />
  <img src="https://img.shields.io/badge/Platform-macOS%20(Apple%20Silicon%20%26%20Intel)-lightgrey.svg?style=flat-square&logo=apple" alt="macOS" />
  <img src="https://img.shields.io/badge/License-MIT-green.svg?style=flat-square" alt="MIT License" />
</p>

---

## 📖 Overview

**OpenMD** is a desktop Markdown and MDX editing environment designed specifically for macOS. By replacing heavy Electron runtimes with a high-performance **Rust backend** and leveraging macOS's built-in **`WKWebView`** via **Tauri v2**, OpenMD delivers instantaneous startup times, minimal memory consumption (~35MB RAM), and deep macOS desktop integration without compromising on rich web-native editing features.

Whether you're taking notes, writing technical documentation with **KaTeX** math formulas, or sketching architectures with **Mermaid.js** diagrams, OpenMD provides an uninterrupted, distraction-free workflow.

---

## ⚡ Architectural Highlights

OpenMD adopts a **Web-Native Hybrid Architecture**:

```
┌────────────────────────────────────────────────────────┐
│                   macOS Desktop Window                 │
│  (Transparent Titlebar, Native Traffic Lights, Menus)  │
└──────────────────────────┬─────────────────────────────┘
                           │
        ┌──────────────────┴──────────────────┐
        ▼                                     ▼
┌───────────────────────────────┐   ┌───────────────────────────────┐
│     Native Rust Backend       │   │   Webview UI (React 19 + TS)  │
│         (Tauri v2)            │   │         (CodeMirror 6)        │
├───────────────────────────────┤   ├───────────────────────────────┤
│ • Native macOS Windowing(tao) │   │ • CodeMirror 6 Editor Engine  │
│ • Recursive Dir Scanner       │IPC│ • Live Split Preview          │
│ • Native File Dialogs (rfd)   │◄─►│ • KaTeX Math Rendering        │
│ • Atomic File I/O & Assets    │   │ • Mermaid.js Vector Diagrams  │
│ • macOS Trash Bin Integration │   │ • GFM Task Checkboxes         │
│ • Dynamic App Menu (menu.rs)  │   │ • Bi-lingual i18n (TR / EN)   │
└───────────────────────────────┘   └───────────────────────────────┘
```

* **Rust Backend:** Handles I/O, secure filesystem operations, native dialogs, and window lifecycle with zero overhead.
* **Tauri v2 Webview:** Uses macOS's native `WKWebView` (via `wry`), avoiding the 180MB+ memory footprint of bundled Chromium.
* **React 19 + TypeScript Frontend:** Delivers rich formatting tools, responsive split-pane navigation, and real-time previews.

---

## 📊 Comparison: OpenMD vs. Traditional Editors

| Metric / Feature | Legacy Electron Editors | Pure Native (Swift/AppKit) | **OpenMD (Rust + Tauri v2)** |
| :--- | :--- | :--- | :--- |
| **Idle Memory (RAM)** | ~180 MB – 320 MB | ~20 MB – 45 MB | **~35 MB – 55 MB** ⚡ |
| **Binary Bundle Size** | ~120 MB – 250 MB | ~10 MB – 25 MB | **~25 MB – 35 MB** 📦 |
| **Startup Speed** | Noticeable delay (1-3s) | Instant (< 300ms) | **Near Instant (< 400ms)** 🚀 |
| **Mermaid & KaTeX** | Supported | Complex / Incomplete | **Full Native Support** ✅ |
| **macOS Integration** | Emulated / Partial | 100% Native | **Native (Window, Menu, Trash)** ✅ |

---

## ✨ Key Features

### ✍️ Next-Gen Markdown & MDX Editing
* **CodeMirror 6 Editor:** Fast syntax highlighting, bracket matching, line numbers, code folding, and multi-cursor support.
* **KaTeX Mathematics:** Real-time formula preview for both inline (`$E = mc^2$`) and block (`$$\int_{0}^{\infty} e^{-x^2} dx$$`) syntax.
* **Mermaid.js Diagrams:** Full vector rendering for flowcharts, sequence diagrams, Gantt charts, class diagrams, state machines, and mindmaps.
* **Interactive GFM Checklists:** Click task checkboxes directly in the live preview to instantly toggle `- [ ]` and `- [x]` in the editor source.

### 🖼️ Smart Clipboard Image Paste
* Copy any image to your clipboard and hit **`Cmd + V`** inside the editor.
* OpenMD automatically decodes the image, writes a collision-free PNG file to `assets/paste-{timestamp}.png`, and inserts the relative Markdown image link at your cursor.

### 📁 Workspace & Explorer
* **Recursive Folder Explorer:** High-speed directory scanner powered by Rust async threads.
* **Strict Markdown Filter:** Prunes asset folders with no markdown documents, focusing solely on `.md` and `.mdx` content.
* **Multi-Tab Document Workspace:** Work across multiple files with session restoration and dirty-state indicators (`*`).
* **Safe Creation:** Intelligent collision-free naming (`Untitled.md`, `Untitled 2.md`).

### 🛡️ Unsaved Changes Safeguard
* Native window close events are intercepted at the Rust level (`on_window_event`).
* If unsaved tabs exist, a modal prompts to **Save**, **Discard**, or **Cancel** before closing tabs, changing workspaces, or quitting.
* Configurable default save path with automatic fallback saving.

### 🌐 Bi-lingual Internationalization (i18n)
* Complete **Türkçe (TR)** and **English (EN)** localization.
* Switching language dynamically updates the UI and recreates the native macOS application menu bar in real-time.

### 📤 Clean Exports
* **Standalone HTML:** Exports self-contained HTML with embedded styles and highlighted syntax.
* **Native PDF:** Generates print-ready PDFs using macOS's built-in WebKit printing subsystem.

---

## ⌨️ macOS Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Cmd + N` | New Document |
| `Cmd + O` | Open Markdown File |
| `Cmd + Shift + O` | Open Workspace Folder |
| `Cmd + S` | Save File |
| `Cmd + Shift + S` | Save As... |
| `Cmd + W` | Close Active Tab |
| `Cmd + P` | Command Palette |
| `Cmd + F` | Search / Find in Editor |
| `Cmd + B` | Bold Formatting (`**text**`) |
| `Cmd + I` | Italic Formatting (`*text*`) |
| `Cmd + K` | Insert Hyperlink |
| `Cmd + E` | Export Document (HTML / PDF) |
| `Cmd + ,` | Settings |
| `Cmd + \` | Toggle Sidebar Visibility |

---

## 🛠️ Tech Stack

* **Backend:** Rust 1.90+, Tauri v2.12, Tokio, Trash, RFD, Walkdir, Serde.
* **Frontend:** React 19, TypeScript 5.8, Vite 6, Lucide Icons.
* **Editor:** CodeMirror 6 (`@codemirror/view`, `@codemirror/state`, `@codemirror/lang-markdown`, `@codemirror/theme-one-dark`).
* **Parsers & Formatters:** `react-markdown`, `remark-gfm`, `remark-math`, `rehype-katex`, `rehype-highlight`, `mermaid`.

---

## 🚀 Getting Started

### Prerequisites

Ensure you have the following installed on macOS:
1. **Node.js** (v18 or higher) — [nodejs.org](https://nodejs.org/)
2. **Rust & Cargo** (v1.90 or higher) — Install via rustup:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
3. **Xcode Command Line Tools**:
   ```bash
   xcode-select --install
   ```

### Installation

Clone the repository and install dependencies:

```bash
git clone https://github.com/aykuttasdelen/OpenMDR.git
cd OpenMDR
npm install
```

### Running in Development

Launch OpenMD with Vite hot-reloading and the native Tauri window:

```bash
npm run dev:tauri
```

*(Optional: Run in web-only browser preview mode with `npm run dev`)*

### Running Automated Tests

```bash
# Frontend unit tests & TypeScript type validation
npm test
npm run typecheck

# Rust backend unit tests
cd src-tauri
cargo test
```

### Building for Production (.app / .dmg)

Create an optimized native macOS application bundle:

```bash
npm run build:tauri
```

The compiled native macOS binary and installer bundles will be generated in:
```
src-tauri/target/release/bundle/macos/
```

---

## 📂 Project Structure

```
OpenMD/
├── src/                    # React 19 + TypeScript Frontend
│   ├── api/                # Tauri IPC Bridge & window.electronAPI adapter
│   ├── components/         # Editor, Preview, Sidebar, TabBar, Modals
│   ├── i18n/               # Turkish & English translation dictionaries
│   ├── types/              # Type definitions (Document, Tabs, FileItem)
│   └── utils/              # Outline extraction, Markdown stats, Exporters
├── src-tauri/              # Native Rust Backend (Tauri v2)
│   ├── capabilities/       # Tauri v2 security & permission policies
│   ├── icons/              # macOS application icons (.icns, .png)
│   ├── src/
│   │   ├── commands/       # Tauri IPC commands (FS, Dialogs, Image, Export)
│   │   ├── lib.rs          # Core Tauri builder, window events & handlers
│   │   ├── main.rs         # Entrypoint
│   │   ├── menu.rs         # Dynamic native macOS application menu
│   │   └── models.rs       # Serde data structs
│   ├── Cargo.toml          # Rust package configuration & dependencies
│   └── tauri.conf.json     # Tauri app configuration & build pipeline
├── test/                   # Automated regression test suite
└── package.json            # Node.js scripts and frontend dependencies
```

---

## 📄 License

Distributed under the **MIT License**. See `LICENSE` for more information.

Crafted with ❤️ for macOS by [Aykut Taşdelen](https://github.com/aykuttasdelen).
