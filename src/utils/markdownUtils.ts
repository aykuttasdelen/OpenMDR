export interface OutlineItem {
  id: string;
  level: number;
  text: string;
  line: number;
}

export interface DocumentStats {
  words: number;
  chars: number;
  lines: number;
  readingTime: number; // in minutes
}

export function extractOutline(markdown: string): OutlineItem[] {
  const lines = markdown.split('\n');
  const items: OutlineItem[] = [];
  let inCodeBlock = false;
  let activeFenceChar = '';
  let activeFenceLen = 0;

  lines.forEach((line, index) => {
    const trimmed = line.trim();
    
    if (!inCodeBlock) {
      const matchFence = trimmed.match(/^(`{3,}|~{3,})/);
      if (matchFence) {
        const fence = matchFence[1];
        const rest = trimmed.slice(fence.length);
        // If fence is opened and closed on the same line, remain outside
        if (rest.includes(fence)) {
          return;
        }
        inCodeBlock = true;
        activeFenceChar = fence[0];
        activeFenceLen = fence.length;
        return;
      }
    } else {
      // Check for closing fence matching or exceeding active fence length
      const matchClose = trimmed.match(/^(`{3,}|~{3,})/);
      if (matchClose && matchClose[1][0] === activeFenceChar && matchClose[1].length >= activeFenceLen) {
        inCodeBlock = false;
        activeFenceChar = '';
        activeFenceLen = 0;
        return;
      }
      return;
    }

    const match = line.match(/^(#{1,6})\s+(.+)$/);
    if (match) {
      const level = match[1].length;
      // Strip trailing optional markdown hashes e.g. "## Heading ##"
      const text = match[2].replace(/\s+#+\s*$/, '').trim();
      const id = `heading-${index}-${text.toLowerCase().replace(/[^\w\s-]/g, '').replace(/\s+/g, '-')}`;
      items.push({
        id,
        level,
        text,
        line: index + 1
      });
    }
  });

  return items;
}

export function calculateStats(markdown: string): DocumentStats {
  if (!markdown || !markdown.trim()) {
    return { words: 0, chars: markdown ? markdown.length : 0, lines: markdown ? markdown.split('\n').length : 1, readingTime: 1 };
  }

  const lines = markdown.split('\n').length;
  const chars = markdown.length;
  const words = markdown.trim().split(/\s+/).filter(Boolean).length;
  const readingTime = Math.max(1, Math.ceil(words / 200));

  return { words, chars, lines, readingTime };
}

/**
 * Toggles a GFM task checkbox in markdown text by index.
 * Aware of code block fences and supports blockquotes (> - [ ]) as well as lists.
 */
export function toggleTaskInMarkdown(markdown: string, targetIndex: number, checked: boolean): string {
  const lines = markdown.split('\n');
  let currentIndex = 0;
  let inCodeBlock = false;
  let activeFenceChar = '';
  let activeFenceLen = 0;
  const taskRegex = /^(\s*(?:>\s*)*[-*+]|\s*(?:>\s*)*\d+\.)\s+\[([ xX])\]/;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const trimmed = line.trim();

    if (!inCodeBlock) {
      const matchFence = trimmed.match(/^(`{3,}|~{3,})/);
      if (matchFence) {
        const fence = matchFence[1];
        const rest = trimmed.slice(fence.length);
        if (!rest.includes(fence)) {
          inCodeBlock = true;
          activeFenceChar = fence[0];
          activeFenceLen = fence.length;
        }
        continue;
      }
    } else {
      const matchClose = trimmed.match(/^(`{3,}|~{3,})/);
      if (matchClose && matchClose[1][0] === activeFenceChar && matchClose[1].length >= activeFenceLen) {
        inCodeBlock = false;
        activeFenceChar = '';
        activeFenceLen = 0;
      }
      continue;
    }

    if (taskRegex.test(line)) {
      if (currentIndex === targetIndex) {
        lines[i] = line.replace(taskRegex, (_match, prefix) => `${prefix} [${checked ? 'x' : ' '}]`);
        return lines.join('\n');
      }
      currentIndex++;
    }
  }

  return markdown;
}

/**
 * Cross-platform path helpers
 */
export function getBasename(filePath: string): string {
  const parts = filePath.split(/[/\\]/);
  return parts.pop() || filePath;
}

export function getDirname(filePath: string): string {
  const lastSlash = Math.max(filePath.lastIndexOf('/'), filePath.lastIndexOf('\\'));
  return lastSlash >= 0 ? filePath.substring(0, lastSlash) : '';
}

export function getImageExtension(mimeType?: string): string {
  if (!mimeType) return 'png';
  const lower = mimeType.toLowerCase();
  if (lower.includes('jpeg') || lower.includes('jpg')) return 'jpg';
  if (lower.includes('gif')) return 'gif';
  if (lower.includes('webp')) return 'webp';
  if (lower.includes('svg')) return 'svg';
  return 'png';
}

/**
 * Resolves a markdown image src to an absolute file:// URL if relative and baseDir is provided.
 * Leaves http(s), data:, blob:, and file: URLs untouched.
 */
export function resolveRelativeImagePath(src: string, baseDir?: string): string {
  if (!src) return '';
  if (/^(https?:\/\/|data:|blob:|file:\/\/)/i.test(src)) {
    return src;
  }
  if (!baseDir) {
    return src;
  }
  const cleanBase = baseDir.replace(/\\/g, '/').replace(/\/+$/, '');
  const cleanSrc = src.replace(/\\/g, '/').replace(/^(\.\/)+/, '').replace(/^\//, '');
  const combined = `${cleanBase}/${cleanSrc}`;
  const cleanPath = combined.startsWith('/') ? combined : `/${combined}`;
  return `file://${encodeURI(cleanPath)}`;
}

/**
 * Validates if a file name is a Markdown file (.md or .mdx)
 */
export function isMarkdownFileName(fileName: string): boolean {
  if (!fileName) return false;
  return /\.(md|mdx)$/i.test(fileName);
}

/**
 * Determines the next available untitled filename avoiding collisions with open tabs.
 * Supports both "İsimsiz", "İsimsiz 2", "İsimsiz2", "Untitled", etc.
 */
export function getNextUntitledTitle(
  existingTitles: string[],
  prefix: string = 'İsimsiz'
): string {
  const normalizedPrefix = prefix.trim();
  const escapedPrefix = normalizedPrefix.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const regex = new RegExp(`^(?:${escapedPrefix}|İsimsiz|Untitled)(?: ?(\\d+))?\\.md$`, 'i');

  const usedNumbers = new Set<number>();
  let hasBase = false;

  for (const title of existingTitles) {
    const cleanTitle = getBasename(title).trim();
    const match = cleanTitle.match(regex);
    if (match) {
      if (match[1]) {
        usedNumbers.add(parseInt(match[1], 10));
      } else {
        hasBase = true;
      }
    }
  }

  if (!hasBase) {
    return `${normalizedPrefix}.md`;
  }

  let counter = 2;
  while (usedNumbers.has(counter)) {
    counter++;
  }
  return `${normalizedPrefix} ${counter}.md`;
}

/**
 * Combines default directory and file name safely
 */
export function getDefaultSaveFilePath(defaultDir: string, fileName: string): string {
  const cleanDir = defaultDir.replace(/[/\\]+$/, '');
  const cleanName = getBasename(fileName);
  const isWindows = defaultDir.includes('\\');
  const separator = isWindows ? '\\' : '/';
  return `${cleanDir}${separator}${cleanName}`;
}

export function hasUnsavedChanges(tabs: { isModified: boolean }[]): boolean {
  return tabs.some(t => t.isModified);
}

export function getUnsavedTabs<T extends { isModified: boolean }>(tabs: T[]): T[] {
  return tabs.filter(t => t.isModified);
}


export function generateStandaloneHTML(title: string, renderedHTML: string): string {
  return `<!DOCTYPE html>
<html lang="tr">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>${title}</title>
  <link rel="preconnect" href="https://fonts.googleapis.com">
  <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
  <link href="https://fonts.googleapis.com/css2?family=Inter:wght@300;400;500;600;700&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet">
  <link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.21/dist/katex.min.css">
  <style>
    body {
      font-family: 'Inter', -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      line-height: 1.7;
      color: #1e293b;
      background-color: #ffffff;
      max-width: 860px;
      margin: 40px auto;
      padding: 0 24px;
    }
    h1, h2, h3, h4, h5, h6 {
      color: #0f172a;
      font-weight: 600;
      line-height: 1.3;
      margin-top: 1.6em;
      margin-bottom: 0.6em;
    }
    h1 { font-size: 2.2rem; border-bottom: 1px solid #e2e8f0; padding-bottom: 0.3em; }
    h2 { font-size: 1.65rem; border-bottom: 1px solid #e2e8f0; padding-bottom: 0.3em; }
    p { margin-bottom: 1.2em; }
    a { color: #2563eb; text-decoration: none; }
    a:hover { text-decoration: underline; }
    blockquote {
      border-left: 4px solid #3b82f6;
      background: #f8fafc;
      padding: 12px 18px;
      margin: 1.4em 0;
      border-radius: 0 8px 8px 0;
      color: #475569;
    }
    /* GitHub Callouts */
    .callout {
      margin: 1.2em 0;
      padding: 14px 18px;
      border-radius: 8px;
      border-left: 4px solid #3b82f6;
      background: #f8fafc;
    }
    .callout-header {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 13px;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.03em;
      margin-bottom: 6px;
    }
    .callout-note { border-left-color: #3b82f6; background: #eff6ff; }
    .callout-note .callout-header { color: #2563eb; }
    .callout-tip { border-left-color: #10b981; background: #ecfdf5; }
    .callout-tip .callout-header { color: #059669; }
    .callout-warning { border-left-color: #f59e0b; background: #fffbeb; }
    .callout-warning .callout-header { color: #d97706; }
    .callout-important { border-left-color: #8b5cf6; background: #f5f3ff; }
    .callout-important .callout-header { color: #7c3aed; }
    .callout-caution { border-left-color: #ef4444; background: #fef2f2; }
    .callout-caution .callout-header { color: #dc2626; }

    /* Code Blocks */
    .code-block-wrapper {
      margin: 1.4em 0;
      border-radius: 8px;
      overflow: hidden;
      border: 1px solid #e2e8f0;
    }
    .code-block-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      background: #1e293b;
      color: #94a3b8;
      padding: 6px 14px;
      font-size: 12px;
      font-family: 'JetBrains Mono', monospace;
    }
    .copy-code-btn {
      display: none !important;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      margin: 1.5em 0;
    }
    th, td {
      border: 1px solid #cbd5e1;
      padding: 10px 14px;
      text-align: left;
    }
    th { background: #f1f5f9; font-weight: 600; }
    tr:nth-child(even) { background: #f8fafc; }
    code {
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.9em;
      background: #f1f5f9;
      padding: 2px 6px;
      border-radius: 4px;
      color: #0284c7;
    }
    pre {
      background: #0f172a;
      color: #f8fafc;
      padding: 16px;
      border-radius: 0 0 8px 8px;
      margin: 0;
      overflow-x: auto;
      font-family: 'JetBrains Mono', monospace;
      font-size: 0.9em;
    }
    pre code {
      background: transparent;
      color: inherit;
      padding: 0;
    }
    ul, ol { padding-left: 24px; margin-bottom: 1.2em; }
    li { margin-bottom: 0.4em; }
    input[type="checkbox"] {
      margin-right: 8px;
    }
    img {
      max-width: 100%;
      height: auto;
      border-radius: 6px;
    }
    @media print {
      body { max-width: 100%; margin: 0; padding: 10mm; }
      .copy-code-btn { display: none !important; }
    }
  </style>
</head>
<body>
  ${renderedHTML}
</body>
</html>`;
}
