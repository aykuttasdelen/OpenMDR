import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { FileItem } from '../types/electron';

export interface OpenFileResult {
  filePath: string;
  content: string;
  name: string;
}

export interface OpenDirectoryResult {
  dirPath: string;
  tree: FileItem[];
}

export const isTauri = (): boolean => {
  return typeof window !== 'undefined' && ('__TAURI_INTERNALS__' in window || '__TAURI__' in window);
};

export const tauriAPI = {
  readFile: async (filePath: string): Promise<string> => {
    return invoke<string>('read_file', { filePath });
  },

  writeFile: async (filePath: string, content: string): Promise<boolean> => {
    return invoke<boolean>('write_file', { filePath, content });
  },

  openFileDialog: async (): Promise<OpenFileResult | null> => {
    return invoke<OpenFileResult | null>('open_file_dialog');
  },

  saveFileDialog: async (
    defaultName?: string,
    content?: string,
    defaultDir?: string
  ): Promise<string | null> => {
    return invoke<string | null>('save_file_dialog', {
      defaultName: defaultName || null,
      defaultDir: defaultDir || null,
      content: content || null,
    });
  },

  openDirectoryDialog: async (
    markdownOnly = true
  ): Promise<OpenDirectoryResult | null> => {
    return invoke<OpenDirectoryResult | null>('open_directory_dialog', {
      markdownOnly,
    });
  },

  readDirectory: async (
    dirPath: string,
    markdownOnly = true
  ): Promise<FileItem[]> => {
    return invoke<FileItem[]>('scan_directory', { dirPath, markdownOnly });
  },

  createFile: async (dirPath: string, fileName: string): Promise<string | null> => {
    return invoke<string>('create_file', { dirPath, fileName });
  },

  createDirectory: async (
    parentPath: string,
    dirName: string
  ): Promise<string | null> => {
    return invoke<string>('create_directory', { parentPath, dirName });
  },

  deleteFile: async (filePath: string): Promise<boolean> => {
    return invoke<boolean>('delete_file', { filePath });
  },

  renameFile: async (oldPath: string, newPath: string): Promise<boolean> => {
    return invoke<boolean>('rename_file', { oldPath, newPath });
  },

  getAvailablePath: async (baseDir: string, fileName: string): Promise<string> => {
    return invoke<string>('get_available_path', { baseDir, fileName });
  },

  saveImage: async (
    baseDir: string,
    base64Data: string,
    mimeType: string
  ): Promise<string | null> => {
    return invoke<string>('save_image', { baseDir, base64Data, mimeType });
  },

  showInFolder: async (filePath: string): Promise<void> => {
    return invoke<void>('show_in_folder', { filePath });
  },

  exportPDF: async (
    htmlContent?: string,
    defaultTitle?: string
  ): Promise<boolean> => {
    if (htmlContent) {
      // In WebKit / Safari, window.print() prints the page or standalone HTML
      const printFrame = document.createElement('iframe');
      printFrame.style.position = 'fixed';
      printFrame.style.right = '0';
      printFrame.style.bottom = '0';
      printFrame.style.width = '0';
      printFrame.style.height = '0';
      printFrame.style.border = '0';
      document.body.appendChild(printFrame);

      const frameDoc = printFrame.contentWindow?.document;
      if (frameDoc) {
        frameDoc.open();
        frameDoc.write(htmlContent);
        frameDoc.close();
        setTimeout(() => {
          printFrame.contentWindow?.focus();
          printFrame.contentWindow?.print();
          setTimeout(() => document.body.removeChild(printFrame), 1000);
        }, 300);
        return true;
      }
    }
    window.print();
    return true;
  },

  saveHTMLFile: async (
    htmlContent: string,
    defaultName?: string
  ): Promise<boolean> => {
    return invoke<boolean>('save_html', {
      htmlContent,
      defaultName: defaultName || null,
    });
  },

  getAppVersion: async (): Promise<string> => {
    return '1.1.0';
  },

  getDocumentsDir: async (): Promise<string> => {
    return invoke<string>('get_default_documents_dir');
  },

  getDefaultDocumentsDir: async (): Promise<string> => {
    return invoke<string>('get_default_documents_dir');
  },

  toggleMaximize: async (): Promise<void> => {
    return invoke<void>('toggle_maximize');
  },

  setLanguage: async (lang: string): Promise<boolean> => {
    try {
      await invoke<void>('set_menu_language', { lang });
      return true;
    } catch (err) {
      console.warn('Failed to set menu language:', err);
      return false;
    }
  },

  setMenuLanguage: async (lang: 'tr' | 'en'): Promise<void> => {
    return invoke<void>('set_menu_language', { lang });
  },

  closeWindow: async (force = false): Promise<void> => {
    return invoke<void>('close_window', { force });
  },

  cancelClose: async (): Promise<void> => {
    return invoke<void>('cancel_close');
  },

  onMenuCommand: (callback: (command: string) => void): (() => void) => {
    let unlistenFn: (() => void) | null = null;
    listen<string>('menu:command', (event) => {
      callback(event.payload);
    }).then((fn) => {
      unlistenFn = fn;
    });

    return () => {
      if (unlistenFn) {
        unlistenFn();
      }
    };
  },

  onWindowClosing: (callback: () => void): (() => void) => {
    let unlistenFn: (() => void) | null = null;
    listen('app:window-closing', () => {
      callback();
    }).then((fn) => {
      unlistenFn = fn;
    });

    return () => {
      if (unlistenFn) {
        unlistenFn();
      }
    };
  },
};

// Initialize bridge on window
if (typeof window !== 'undefined') {
  if (isTauri()) {
    (window as any).electronAPI = tauriAPI;
    (window as any).tauriAPI = tauriAPI;
  }
}
