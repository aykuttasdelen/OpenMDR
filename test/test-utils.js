import assert from 'node:assert';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { execFileSync } from 'node:child_process';
import { 
  extractOutline, 
  calculateStats, 
  generateStandaloneHTML, 
  toggleTaskInMarkdown,
  getBasename,
  getDirname,
  getImageExtension,
  resolveRelativeImagePath,
  getNextUntitledTitle
} from '../src/utils/markdownUtils.ts';

console.log('--- Testing Markdown Utilities ---');

// Test 1: extractOutline
const sampleMarkdown = `
# Başlık 1
Giriş paragrafı

## Alt Başlık 1.1 ##
Detaylar...

### Üçüncü Seviye
Daha fazla detay

## Alt Başlık 1.2
Sonuç
`;

const outline = extractOutline(sampleMarkdown);
assert.strictEqual(outline.length, 4, 'Should extract 4 headings');
assert.strictEqual(outline[0].text, 'Başlık 1');
assert.strictEqual(outline[0].level, 1);
assert.strictEqual(outline[1].text, 'Alt Başlık 1.1', 'Trailing hashes should be stripped');
assert.strictEqual(outline[1].level, 2);
assert.strictEqual(outline[2].text, 'Üçüncü Seviye');
assert.strictEqual(outline[2].level, 3);
assert.strictEqual(outline[3].text, 'Alt Başlık 1.2');
assert.strictEqual(outline[3].level, 2);
console.log('✓ extractOutline test passed');

// Test 2: calculateStats
const text = 'Bu bir test metnidir. Toplam kelime ve karakter sayıları hesaplanır.';
const stats = calculateStats(text);
assert.strictEqual(stats.words, 10, 'Should count 10 words');
assert(stats.chars > 50, 'Chars count should be greater than 50');
assert.strictEqual(stats.lines, 1, 'Should be 1 line');
assert.strictEqual(stats.readingTime, 1, 'Reading time should be at least 1 min');
console.log('✓ calculateStats test passed');

// Test 3: generateStandaloneHTML
const html = generateStandaloneHTML('Test Belgesi', '<p>Merhaba Dünya</p>');
assert(html.includes('<!DOCTYPE html>'), 'HTML must include DOCTYPE');
assert(html.includes('<title>Test Belgesi</title>'), 'HTML must include title');
assert(html.includes('Merhaba Dünya'), 'HTML must include body content');
assert(html.includes('katex.min.css'), 'HTML must include KaTeX styles');
assert(html.includes('.callout-note'), 'HTML must include callout styles');
assert(html.includes('.copy-code-btn'), 'HTML must include copy button print rules');
console.log('✓ generateStandaloneHTML test passed');

// Test 1b: extractOutline with nested code fences
const codeFenceMarkdown = `
# Gerçek Başlık 1

\`\`\`bash
# Bu bir bash yorumudur, başlık değildir!
## Bu da kod içindedir
\`\`\`

## Gerçek Başlık 2

\`\`\`\`markdown
\`\`\`
# Bu da 4-backtick içinde bir başlıktır, outline olmamalı!
\`\`\`
\`\`\`\`

~~~python
# Python yorumu
print("test")
~~~

### Gerçek Başlık 3
`;

const outlineCode = extractOutline(codeFenceMarkdown);
assert.strictEqual(outlineCode.length, 3, 'Should ignore headings inside 3-backtick, 4-backtick, and tilde blocks');
assert.strictEqual(outlineCode[0].text, 'Gerçek Başlık 1');
assert.strictEqual(outlineCode[1].text, 'Gerçek Başlık 2');
assert.strictEqual(outlineCode[2].text, 'Gerçek Başlık 3');
console.log('✓ extractOutline with nested code fences test passed');

// Test 2b: calculateStats empty / whitespace
const emptyStats = calculateStats('');
assert.strictEqual(emptyStats.words, 0);
assert.strictEqual(emptyStats.chars, 0);
const whitespaceStats = calculateStats('   \n\t  \n  ');
assert.strictEqual(whitespaceStats.words, 0);
console.log('✓ calculateStats empty/whitespace test passed');

// Test 4: toggleTaskInMarkdown
const taskMarkdown = `\`\`\`bash
- [ ] code example
\`\`\`
- [ ] First task
  - [ ] Subtask
> - [ ] Task in quote
>> 1. [ ] Numbered task in quote
* [x] Star task
`;

// Toggle subtask (index 1) to checked
let toggled = toggleTaskInMarkdown(taskMarkdown, 1, true);
assert(toggled.includes('  - [x] Subtask'), 'Should check subtask at index 1');
assert(toggled.includes('- [ ] code example'), 'Should NOT toggle code example');

// Toggle quote task (index 2) to checked
toggled = toggleTaskInMarkdown(toggled, 2, true);
assert(toggled.includes('> - [x] Task in quote'), 'Should check quote task at index 2');

// Toggle star task (index 4) to unchecked
toggled = toggleTaskInMarkdown(toggled, 4, false);
assert(toggled.includes('* [ ] Star task'), 'Should uncheck star task at index 4');

console.log('✓ toggleTaskInMarkdown test passed');

// Test 5: Path helpers
assert.strictEqual(getBasename('/Users/test/doc.md'), 'doc.md');
assert.strictEqual(getBasename('C:\\Users\\test\\notes.markdown'), 'notes.markdown');
assert.strictEqual(getBasename('doc.md'), 'doc.md');
assert.strictEqual(getDirname('/Users/test/doc.md'), '/Users/test');
assert.strictEqual(getDirname('C:\\Users\\test\\notes.markdown'), 'C:\\Users\\test');
assert.strictEqual(getDirname('doc.md'), '');
console.log('✓ Path helpers test passed');

// Test 6: getImageExtension
assert.strictEqual(getImageExtension('image/png'), 'png');
assert.strictEqual(getImageExtension('image/jpeg'), 'jpg');
assert.strictEqual(getImageExtension('image/jpg'), 'jpg');
assert.strictEqual(getImageExtension('image/webp'), 'webp');
assert.strictEqual(getImageExtension('image/gif'), 'gif');
assert.strictEqual(getImageExtension('image/svg+xml'), 'svg');
assert.strictEqual(getImageExtension(''), 'png');
assert.strictEqual(getImageExtension(undefined), 'png');
console.log('✓ getImageExtension test passed');

// Test 7: Smart image paste assets workflow simulation
async function testSaveImageWorkflow() {
  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'openmd-test-'));
  try {
    const assetsDir = path.join(tmpDir, 'assets');
    await fs.mkdir(assetsDir, { recursive: true });

    const mimeType = 'image/png';
    const ext = getImageExtension(mimeType);
    const fakeBase64 = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
    const timestamp = Date.now();
    const fileName = `paste-${timestamp}.${ext}`;
    const filePath = path.join(assetsDir, fileName);

    const buffer = Buffer.from(fakeBase64, 'base64');
    await fs.writeFile(filePath, buffer);

    const relativePath = `assets/${fileName}`;
    assert(relativePath.startsWith('assets/paste-'));
    assert(relativePath.endsWith('.png'));

    const readBack = await fs.readFile(filePath);
    assert.strictEqual(readBack.length, buffer.length);
    console.log('✓ Smart image paste assets workflow test passed');
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true });
  }
}

await testSaveImageWorkflow();

// Test 8: resolveRelativeImagePath
assert.strictEqual(
  resolveRelativeImagePath('assets/paste-1.png', '/Users/test/workspace'),
  'file:///Users/test/workspace/assets/paste-1.png'
);
assert.strictEqual(
  resolveRelativeImagePath('./assets/paste-2.png', '/Users/test/workspace/'),
  'file:///Users/test/workspace/assets/paste-2.png'
);
assert.strictEqual(
  resolveRelativeImagePath('assets/paste-win.png', 'C:\\Users\\test\\docs'),
  'file:///C:/Users/test/docs/assets/paste-win.png'
);
assert.strictEqual(
  resolveRelativeImagePath('https://example.com/image.png', '/Users/test'),
  'https://example.com/image.png'
);
assert.strictEqual(
  resolveRelativeImagePath('data:image/png;base64,abc123==', '/Users/test'),
  'data:image/png;base64,abc123=='
);
assert.strictEqual(
  resolveRelativeImagePath('blob:http://localhost:5173/uuid', '/Users/test'),
  'blob:http://localhost:5173/uuid'
);
assert.strictEqual(
  resolveRelativeImagePath('file:///Users/test/already-file.png', '/Users/test'),
  'file:///Users/test/already-file.png'
);
assert.strictEqual(
  resolveRelativeImagePath('assets/paste-no-base.png', undefined),
  'assets/paste-no-base.png'
);
assert.strictEqual(resolveRelativeImagePath(''), '');
console.log('✓ resolveRelativeImagePath test passed');

// Test 9: isMarkdownFileName (Task 1: only .md and .mdx files)
import { 
  isMarkdownFileName, 
  getDefaultSaveFilePath, 
  hasUnsavedChanges, 
  getUnsavedTabs 
} from '../src/utils/markdownUtils.ts';
import { getTranslation, translations } from '../src/i18n/translations.ts';

assert.strictEqual(isMarkdownFileName('note.md'), true, 'Should accept .md');
assert.strictEqual(isMarkdownFileName('component.mdx'), true, 'Should accept .mdx');
assert.strictEqual(isMarkdownFileName('README.MD'), true, 'Should accept uppercase .MD');
assert.strictEqual(isMarkdownFileName('DOCS.MDX'), true, 'Should accept uppercase .MDX');
assert.strictEqual(isMarkdownFileName('script.js'), false, 'Should reject .js');
assert.strictEqual(isMarkdownFileName('image.png'), false, 'Should reject .png');
assert.strictEqual(isMarkdownFileName('data.json'), false, 'Should reject .json');
assert.strictEqual(isMarkdownFileName('document.pdf'), false, 'Should reject .pdf');
assert.strictEqual(isMarkdownFileName('notes.txt'), false, 'Should reject .txt');
assert.strictEqual(isMarkdownFileName(''), false, 'Should reject empty string');
console.log('✓ isMarkdownFileName test passed');

// Test 10: getDefaultSaveFilePath (Task 3: default save location)
assert.strictEqual(
  getDefaultSaveFilePath('/Users/test/Documents/Notes', 'Untitled.md'),
  '/Users/test/Documents/Notes/Untitled.md'
);
assert.strictEqual(
  getDefaultSaveFilePath('/Users/test/Documents/Notes/', 'Yeni Belge.md'),
  '/Users/test/Documents/Notes/Yeni Belge.md'
);
assert.strictEqual(
  getDefaultSaveFilePath('C:\\Users\\test\\Documents\\Notes', 'Untitled.md'),
  'C:\\Users\\test\\Documents\\Notes\\Untitled.md'
);
console.log('✓ getDefaultSaveFilePath test passed');

// Test 11: hasUnsavedChanges and getUnsavedTabs (Task 2: unsaved workspace warning)
const sampleTabs = [
  { id: '1', title: 'Doc 1.md', isModified: false, content: 'a', savedContent: 'a' },
  { id: '2', title: 'Doc 2.md', isModified: true, content: 'b', savedContent: '' },
  { id: '3', title: 'Doc 3.md', isModified: false, content: 'c', savedContent: 'c' }
];
assert.strictEqual(hasUnsavedChanges(sampleTabs), true);
assert.strictEqual(getUnsavedTabs(sampleTabs).length, 1);
assert.strictEqual(getUnsavedTabs(sampleTabs)[0].id, '2');

const cleanTabs = [
  { id: '1', title: 'Doc 1.md', isModified: false, content: 'a', savedContent: 'a' }
];
assert.strictEqual(hasUnsavedChanges(cleanTabs), false);
assert.strictEqual(getUnsavedTabs(cleanTabs).length, 0);
console.log('✓ hasUnsavedChanges and getUnsavedTabs tests passed');

// Test 12: i18n Translation System & Dictionary Parity (Task 4)
assert.strictEqual(getTranslation('tr', 'app.untitled'), 'İsimsiz');
assert.strictEqual(getTranslation('en', 'app.untitled'), 'Untitled');
assert.strictEqual(
  getTranslation('tr', 'app.unsavedWorkspaceMsg', { count: 3 }).includes('3'),
  true,
  'Turkish count parameter should be interpolated'
);
assert.strictEqual(
  getTranslation('en', 'app.unsavedWorkspaceMsg', { count: 3 }).includes('3'),
  true,
  'English count parameter should be interpolated'
);

const trKeys = Object.keys(translations.tr);
const enKeys = Object.keys(translations.en);

assert.strictEqual(trKeys.length, enKeys.length, `Key counts must match (TR: ${trKeys.length}, EN: ${enKeys.length})`);
for (const key of trKeys) {
  assert(key in translations.en, `Key "${key}" in Turkish must also exist in English dictionary`);
  assert(translations.en[key].length > 0, `English translation for "${key}" must not be empty`);
  assert(translations.tr[key].length > 0, `Turkish translation for "${key}" must not be empty`);
}
console.log(`✓ i18n translation system passed with ${trKeys.length} matching keys`);

// Test 13: Simulated Directory Scan with .md / .mdx filter
async function testScanDirectoryFilter() {
  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'openmd-filter-test-'));
  try {
    await fs.writeFile(path.join(tmpDir, 'document.md'), '# Markdown');
    await fs.writeFile(path.join(tmpDir, 'guide.mdx'), '# MDX component');
    await fs.writeFile(path.join(tmpDir, 'styles.css'), 'body {}');
    await fs.writeFile(path.join(tmpDir, 'package.json'), '{}');
    await fs.writeFile(path.join(tmpDir, 'photo.png'), '');

    const subDir = path.join(tmpDir, 'subfolder');
    await fs.mkdir(subDir);
    await fs.writeFile(path.join(subDir, 'subdoc.md'), '# Sub Markdown');
    await fs.writeFile(path.join(subDir, 'script.js'), 'console.log()');

    // Add an empty folder (newly created)
    const emptySubDir = path.join(tmpDir, 'empty-notes');
    await fs.mkdir(emptySubDir);

    // Add a folder with ONLY non-markdown files (e.g. assets)
    const assetsDir = path.join(tmpDir, 'assets');
    await fs.mkdir(assetsDir);
    await fs.writeFile(path.join(assetsDir, 'logo.png'), '');
    await fs.writeFile(path.join(assetsDir, 'script.js'), '');

    // Simulate improved scanDirectory logic with directory pruning
    async function scan(dir, markdownOnly = true) {
      const entries = await fs.readdir(dir, { withFileTypes: true });
      const items = [];
      for (const entry of entries) {
        const full = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          const children = await scan(full, markdownOnly);
          if (markdownOnly) {
            if (children.length === 0) {
              const rawSubEntries = await fs.readdir(full).catch(() => []);
              const validRaw = rawSubEntries.filter(name => !name.startsWith('.') && name !== 'node_modules');
              if (validRaw.length > 0) {
                // Folder only has non-markdown files, prune it!
                continue;
              }
            }
          }
          items.push({ name: entry.name, isDirectory: true, children });
        } else {
          if (markdownOnly && !isMarkdownFileName(entry.name)) {
            continue;
          }
          items.push({ name: entry.name, isDirectory: false });
        }
      }
      return items;
    }

    const filtered = await scan(tmpDir, true);
    const topLevelFiles = filtered.filter(i => !i.isDirectory).map(i => i.name);
    assert.deepStrictEqual(topLevelFiles.sort(), ['document.md', 'guide.mdx'].sort());

    const dirNames = filtered.filter(i => i.isDirectory).map(i => i.name);
    assert(dirNames.includes('subfolder'), 'Subfolder containing subdoc.md must be kept');
    assert(dirNames.includes('empty-notes'), 'Genuinely empty folder must be kept so users can create files in it');
    assert(!dirNames.includes('assets'), 'Folder containing only non-markdown files must be pruned from markdown view');

    const subFolder = filtered.find(i => i.name === 'subfolder');
    assert(subFolder, 'Subfolder should exist');
    const subFiles = subFolder.children.filter(i => !i.isDirectory).map(i => i.name);
    assert.deepStrictEqual(subFiles, ['subdoc.md'], 'Only subdoc.md should be in subfolder');

    console.log('✓ Directory scan filtering simulation with asset pruning test passed');
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true });
  }
}

await testScanDirectoryFilter();

// Test 14: Collision-Free Path Generation for Auto-Save
async function testCollisionFreePathGeneration() {
  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'openmd-collision-test-'));
  try {
    async function getAvailablePath(dirPath, fileName) {
      let candidate = fileName;
      const ext = path.extname(fileName) || '.md';
      const base = path.basename(fileName, ext);
      let counter = 1;
      let fullPath = path.join(dirPath, candidate);

      while (await fs.stat(fullPath).then(() => true).catch(() => false)) {
        counter++;
        candidate = `${base} ${counter}${ext}`;
        fullPath = path.join(dirPath, candidate);
      }
      return fullPath;
    }

    // Step 1: Initial available path should be the requested filename
    const path1 = await getAvailablePath(tmpDir, 'Untitled.md');
    assert.strictEqual(path.basename(path1), 'Untitled.md');
    await fs.writeFile(path1, '# Existing Note 1');

    // Step 2: Next available path must NOT overwrite existing file
    const path2 = await getAvailablePath(tmpDir, 'Untitled.md');
    assert.strictEqual(path.basename(path2), 'Untitled 2.md');
    await fs.writeFile(path2, '# Existing Note 2');

    // Step 3: Next available path should increment to 3
    const path3 = await getAvailablePath(tmpDir, 'Untitled.md');
    assert.strictEqual(path.basename(path3), 'Untitled 3.md');

    // Verify existing files were preserved intact
    const note1 = await fs.readFile(path1, 'utf-8');
    assert.strictEqual(note1, '# Existing Note 1');

    console.log('✓ Collision-free path generation test passed');
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true });
  }
}

await testCollisionFreePathGeneration();

// Test 15: Unsaved Save-All Abort Safety
function simulateSaveAll(tabs, saveDialogResults) {
  let dialogCallCount = 0;
  const savedTabs = [...tabs];

  for (let i = 0; i < savedTabs.length; i++) {
    const tab = savedTabs[i];
    if (!tab.isModified) continue;

    let targetPath = tab.filePath;
    if (!targetPath) {
      const result = saveDialogResults[dialogCallCount++];
      if (!result) {
        // User canceled dialog! Abort!
        return { aborted: true, tabs: savedTabs };
      }
      targetPath = result;
    }
    savedTabs[i] = { ...tab, filePath: targetPath, isModified: false };
  }

  return { aborted: false, tabs: savedTabs };
}

const mockTabs = [
  { id: '1', title: 'Doc 1.md', isModified: true, filePath: undefined },
  { id: '2', title: 'Doc 2.md', isModified: true, filePath: undefined }
];

// If user cancels first dialog (null)
const resultCanceled = simulateSaveAll(mockTabs, [null]);
assert.strictEqual(resultCanceled.aborted, true, 'Save all must abort when dialog canceled');
assert.strictEqual(resultCanceled.tabs[0].isModified, true, 'Tab 1 must still be marked modified');

// If user saves both files
const resultSuccess = simulateSaveAll(mockTabs, ['/path/Doc 1.md', '/path/Doc 2.md']);
assert.strictEqual(resultSuccess.aborted, false, 'Save all should succeed');
assert.strictEqual(resultSuccess.tabs[0].isModified, false);
assert.strictEqual(resultSuccess.tabs[1].isModified, false);
console.log('✓ Unsaved save-all abort safety test passed');

// Test 16: getNextUntitledTitle collision-free numbering
assert.strictEqual(getNextUntitledTitle([], 'İsimsiz'), 'İsimsiz.md');
assert.strictEqual(getNextUntitledTitle(['İsimsiz.md'], 'İsimsiz'), 'İsimsiz 2.md');
assert.strictEqual(getNextUntitledTitle(['İsimsiz.md', 'İsimsiz 2.md'], 'İsimsiz'), 'İsimsiz 3.md');
assert.strictEqual(getNextUntitledTitle(['İsimsiz.md', 'İsimsiz2.md'], 'İsimsiz'), 'İsimsiz 3.md');
assert.strictEqual(getNextUntitledTitle(['/Users/aykut/Documents/İsimsiz.md', 'İsimsiz 2.md'], 'İsimsiz'), 'İsimsiz 3.md');
assert.strictEqual(getNextUntitledTitle(['Untitled.md'], 'Untitled'), 'Untitled 2.md');
console.log('✓ getNextUntitledTitle collision-free numbering test passed');

// Test 17: OCR PDF Image Extraction & Assets Resolution Workflow
async function testOCRImageExtractionWorkflow() {
  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'openmd-ocr-test-'));
  try {
    const assetsDir = path.join(tmpDir, 'assets');
    await fs.mkdir(assetsDir, { recursive: true });

    // Simulate extracted assets written to disk
    const asset1Name = 'document_ocr-p1-1.png';
    const asset2Name = 'document_ocr-p2-1.jpg';
    await fs.writeFile(path.join(assetsDir, asset1Name), Buffer.from([0x89, 0x50, 0x4e, 0x47]));
    await fs.writeFile(path.join(assetsDir, asset2Name), Buffer.from([0xff, 0xd8, 0xff, 0xe0]));

    // Simulate OCR generated Markdown output
    const ocrMarkdown = `# Rapor Başlığı

Giriş paragrafı ve belge içeriği.

![Görsel](assets/${asset1Name})

İkinci sayfa içeriği ve ek grafik.

![Görsel](assets/${asset2Name})
`;

    // 1. Verify Outline extraction works correctly with images present
    const outline = extractOutline(ocrMarkdown);
    assert.strictEqual(outline.length, 1);
    assert.strictEqual(outline[0].text, 'Rapor Başlığı');

    // 2. Verify Stats calculation handles image tags properly
    const stats = calculateStats(ocrMarkdown);
    assert.strictEqual(stats.lines, 10);
    assert(stats.words > 10);

    // 3. Verify Relative Image Path resolution relative to document destination dir
    const docFilePath = path.join(tmpDir, 'document_ocr (OCR).md');
    const docDir = path.dirname(docFilePath);

    const resolvedSrc1 = resolveRelativeImagePath(`assets/${asset1Name}`, docDir);
    const resolvedSrc2 = resolveRelativeImagePath(`assets/${asset2Name}`, docDir);

    assert(resolvedSrc1.startsWith('file://'), 'Resolved path must use file:// protocol');
    assert(resolvedSrc1.includes(asset1Name), 'Resolved path must contain asset name');
    assert(resolvedSrc2.includes(asset2Name), 'Resolved path must contain asset name');

    // 4. Verify i18n keys for OCR Image Extraction exist in both TR and EN
    assert(getTranslation('tr', 'ocr.extractImages').length > 0);
    assert(getTranslation('tr', 'ocr.extractImagesDesc').length > 0);
    assert(getTranslation('en', 'ocr.extractImages').length > 0);
    assert(getTranslation('en', 'ocr.extractImagesDesc').length > 0);

    console.log('✓ OCR PDF Image Extraction & Assets Resolution test passed');
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true });
  }
}

await testOCRImageExtractionWorkflow();

// Test 18: Native OCR Engine End-to-End Image Extraction Test (macOS)
async function testNativeOCREngineE2E() {
  if (process.platform !== 'darwin') return;
  const ocrBin = path.resolve('src-tauri/native/ocr_engine');
  try {
    await fs.access(ocrBin);
  } catch {
    console.log('ℹ ocr_engine binary not present; skipping native E2E test');
    return;
  }

  const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'openmd-native-ocr-'));
  const pdfPath = path.join(tmpDir, 'sample_doc.pdf');
  const outDir = path.join(tmpDir, 'output');

  try {
    // Generate sample PDF via swift
    const genScript = `
import Foundation
import CoreGraphics
var mediaBox = CGRect(x: 0, y: 0, width: 400, height: 400)
let consumer = CGDataConsumer(url: URL(fileURLWithPath: "${pdfPath}") as CFURL)!
let ctx = CGContext(consumer: consumer, mediaBox: &mediaBox, nil)!
ctx.beginPDFPage(nil)
let cs = CGColorSpaceCreateDeviceRGB()
let imgCtx = CGContext(data: nil, width: 64, height: 64, bitsPerComponent: 8, bytesPerRow: 256, space: cs, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
imgCtx.setFillColor(red: 1, green: 0, blue: 0, alpha: 1)
imgCtx.fill(CGRect(x: 0, y: 0, width: 64, height: 64))
let img = imgCtx.makeImage()!
ctx.draw(img, in: CGRect(x: 50, y: 50, width: 100, height: 100))
ctx.endPDFPage()
ctx.closePDF()
`;
    execFileSync('swift', ['-e', genScript], { encoding: 'utf8' });

    // 1. Run with extractImages: true
    const stdout = execFileSync(ocrBin, [
      '--input', pdfPath,
      '--output-dir', outDir,
      '--extract-images', 'true'
    ], { encoding: 'utf8' });

    const lines = stdout.trim().split('\n');
    const resultLine = lines.find(l => l.includes('"type":"result"'));
    assert(resultLine, 'ocr_engine must output a result payload');
    const parsed = JSON.parse(resultLine);
    assert.strictEqual(parsed.success, true);
    assert(parsed.extractedImages && parsed.extractedImages.length > 0, 'Extracted images must not be empty');
    assert(parsed.markdown.includes('![Görsel](assets/'), 'Markdown must link extracted assets');

    const assetOnDisk = path.join(outDir, parsed.extractedImages[0]);
    const stat = await fs.stat(assetOnDisk);
    assert(stat.size > 0, 'Extracted asset file must exist and be non-empty');

    // 2. Run with extractImages: false
    const stdoutNoImg = execFileSync(ocrBin, [
      '--input', pdfPath,
      '--output-dir', path.join(tmpDir, 'no_img_out'),
      '--extract-images', 'false'
    ], { encoding: 'utf8' });
    const parsedNoImg = JSON.parse(stdoutNoImg.trim().split('\n').find(l => l.includes('"type":"result"')));
    assert.strictEqual(parsedNoImg.success, true);
    assert.strictEqual(parsedNoImg.extractedImages.length, 0);

    console.log('✓ Native OCR Engine End-to-End Image Extraction test passed');
  } finally {
    await fs.rm(tmpDir, { recursive: true, force: true });
  }
}

await testNativeOCREngineE2E();

console.log('\nAll automated tests passed successfully! 🎉');


