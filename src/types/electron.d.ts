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

export interface ElectronAPI {
  readFile: (filePath: string) => Promise<string>;
  writeFile: (filePath: string, content: string) => Promise<boolean>;
  saveImage: (baseDir: string, base64Data: string, mimeType: string) => Promise<string>;
  openFileDialog: () => Promise<{ filePath: string; content: string; name: string } | null>;
  saveFileDialog: (defaultName?: string, content?: string, defaultDir?: string) => Promise<string | null>;
  openDirectoryDialog: (markdownOnly?: boolean) => Promise<{ dirPath: string; tree: FileItem[] } | null>;
  readDirectory: (dirPath: string, markdownOnly?: boolean) => Promise<FileItem[]>;
  createFile: (dirPath: string, fileName: string) => Promise<string | null>;
  createDirectory: (parentPath: string, dirName: string) => Promise<string | null>;
  deleteFile: (filePath: string) => Promise<boolean>;
  renameFile: (oldPath: string, newPath: string) => Promise<boolean>;
  showInFolder: (filePath: string) => Promise<void>;
  getAvailablePath: (dirPath: string, fileName: string) => Promise<string>;
  exportPDF: (htmlContent?: string, defaultTitle?: string) => Promise<boolean>;
  saveHTMLFile: (htmlContent: string, defaultName?: string) => Promise<boolean>;
  getAppVersion: () => Promise<string>;
  getDocumentsDir?: () => Promise<string>;
  getDefaultDocumentsDir?: () => Promise<string>;
  toggleMaximize?: () => Promise<void>;
  setLanguage: (lang: string) => Promise<boolean>;
  closeWindow: (force?: boolean) => Promise<void>;
  cancelClose: () => Promise<boolean>;
  onMenuCommand: (callback: (command: string) => void) => () => void;
  onWindowClosing: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    electronAPI?: ElectronAPI;
  }
}
