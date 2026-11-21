export interface FileItem {
  name: string;
  path: string;
  isDirectory: boolean;
  children?: FileItem[];
  size?: number;
}

export interface Tab {
  id: string;
  title: string;
  filePath?: string;
  content: string;
  savedContent: string;
  isModified: boolean;
}

export interface OutlineItem {
  id: string;
  level: number;
  text: string;
  line: number;
}

export type ViewMode = 'editor' | 'split' | 'preview';
export type ThemeMode = 'dark' | 'light' | 'auto';

export interface OCRProgress {
  type: string;
  page: number;
  totalPages: number;
  percent: number;
}

export interface OCRPage {
  pageNumber: number;
  isOcr: boolean;
  text: string;
  images?: string[];
}

export interface OCRResult {
  success: boolean;
  totalPages: number;
  processedPages: number;
  markdown: string;
  pages: OCRPage[];
  extractedImages?: string[];
  error?: string;
}

export interface OCROptions {
  pdfPath: string;
  pages?: string;
  languages?: string;
  mode?: 'hybrid' | 'ocr' | 'text';
  extractImages?: boolean;
  outputDir?: string;
  subfolder?: string;
  imagePlacement?: 'folderOnly' | 'pageEnd' | 'docEnd' | 'inline';
}

export type SaveSource = 'manual' | 'autosave' | 'initial';
export type HistoryTrigger = 'initial' | 'manual' | 'autosave' | 'pre_restore';

export interface HistorySettings {
  enabled: boolean;
  maxVersionsPerFile: number;
  autosaveIntervalMinutes: number;
}

export interface SnapshotSummary {
  id: number;
  fileId: number;
  createdAt: number;
  trigger: HistoryTrigger;
  sizeBytes: number;
  compressedBytes: number;
  lineCount: number;
  wordCount: number;
}

export interface DeletedFileSummary {
  fileId: number;
  filePath: string;
  fileName: string;
  updatedAt: number;
  snapshotCount: number;
  latestSnapshotId: number;
  latestSizeBytes: number;
}

export interface DiffLine {
  tag: 'equal' | 'insert' | 'delete';
  oldLineNo: number | null;
  newLineNo: number | null;
  content: string;
}

export interface DiffResult {
  lines: DiffLine[];
  addedCount: number;
  removedCount: number;
  unchangedCount: number;
  truncated: boolean;
}

export interface HistoryStats {
  totalFiles: number;
  totalSnapshots: number;
  dbSizeBytes: number;
}

export interface RecoverFileResult {
  filePath: string;
  fileName: string;
  content: string;
}

export interface ElectronAPI {
  readFile: (filePath: string) => Promise<string>;
  writeFile: (filePath: string, content: string, saveSource?: SaveSource) => Promise<boolean>;
  saveImage: (baseDir: string, base64Data: string, mimeType: string) => Promise<string>;
  readImageBase64?: (filePath: string, baseDir?: string) => Promise<string>;
  openFileDialog: () => Promise<{ filePath: string; content: string; name: string } | null>;
  openPdfDialog?: () => Promise<string | null>;
  saveFileDialog: (defaultName?: string, content?: string, defaultDir?: string) => Promise<string | null>;
  openDirectoryDialog: (markdownOnly?: boolean) => Promise<{ dirPath: string; tree: FileItem[] } | null>;
  readDirectory: (dirPath: string, markdownOnly?: boolean) => Promise<FileItem[]>;
  createFile: (dirPath: string, fileName: string) => Promise<string>;
  createDirectory: (parentPath: string, dirName: string) => Promise<string>;
  deleteFile: (filePath: string) => Promise<boolean>;
  renameFile: (oldPath: string, newPath: string) => Promise<boolean>;
  showInFolder: (filePath: string) => Promise<void>;
  getAvailablePath: (dirPath: string, fileName: string) => Promise<string>;
  exportPDF: (htmlContent?: string, defaultTitle?: string) => Promise<boolean>;
  saveHTMLFile: (htmlContent: string, defaultName?: string, defaultDir?: string | null) => Promise<string | null>;
  getAppVersion: () => Promise<string>;
  getDocumentsDir?: () => Promise<string>;
  getDefaultDocumentsDir?: () => Promise<string>;
  toggleMaximize?: () => Promise<void>;
  setWindowTheme?: (theme: ThemeMode) => Promise<void>;
  setLanguage: (lang: string) => Promise<boolean>;
  closeWindow: (force?: boolean) => Promise<void>;
  cancelClose: () => Promise<void>;
  processPdfOCR?: (options: OCROptions) => Promise<OCRResult>;
  onOCRProgress?: (callback: (progress: OCRProgress) => void) => () => void;
  onMenuCommand: (callback: (command: string) => void) => () => void;
  onWindowClosing: (callback: () => void) => () => void;
  onFileDrop?: (callback: (paths: string[]) => void) => () => void;
  historyGetSettings: () => Promise<HistorySettings>;
  historyUpdateSettings: (settings: HistorySettings) => Promise<HistorySettings>;
  historyListSnapshots: (filePath: string) => Promise<SnapshotSummary[]>;
  historyListSnapshotsByFileId: (fileId: number) => Promise<SnapshotSummary[]>;
  historyGetSnapshotContent: (snapshotId: number) => Promise<string>;
  historyDiffSnapshot: (snapshotId: number, currentContent: string) => Promise<DiffResult>;
  historyRestoreSnapshot: (
    filePath: string,
    snapshotId: number,
    currentContent: string
  ) => Promise<string>;
  historyListDeletedFiles: () => Promise<DeletedFileSummary[]>;
  historyRecoverFile: (
    fileId: number,
    snapshotId: number,
    targetDir?: string | null
  ) => Promise<RecoverFileResult>;
  historyDeleteFileHistory: (fileId: number) => Promise<boolean>;
  historyClearAll: () => Promise<boolean>;
  historyGetStats: () => Promise<HistoryStats>;
}

declare global {
  interface Window {
    electronAPI?: ElectronAPI;
  }
}
