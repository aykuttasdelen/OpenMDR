import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import {
  FileItem,
  OCRProgress,
  OCRResult,
  OCROptions,
  SaveSource,
  HistorySettings,
  SnapshotSummary,
  DeletedFileSummary,
  DiffResult,
  HistoryStats,
  RecoverFileResult,
  DriveSettings,
  DriveStatus,
  DriveRemoteFile,
  DriveRestoreResult,
  DriveBatchRestoreResult,
  DriveFlushResult,
} from '../types/electron';
import { escapeHtml } from '../utils/markdownUtils';

function subscribeEvent<T>(
  eventName: string,
  handler: (payload: T) => void
): () => void {
  let cancelled = false;
  let unlistenFn: (() => void) | null = null;

  listen<T>(eventName, (event) => {
    handler(event.payload);
  }).then((fn) => {
    if (cancelled) {
      fn();
    } else {
      unlistenFn = fn;
    }
  });

  return () => {
    cancelled = true;
    if (unlistenFn) {
      unlistenFn();
      unlistenFn = null;
    }
  };
}

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

  writeFile: async (
    filePath: string,
    content: string,
    saveSource?: SaveSource
  ): Promise<boolean> => {
    return invoke<boolean>('write_file', {
      filePath,
      content,
      saveSource: saveSource || null,
    });
  },

  openFileDialog: async (): Promise<OpenFileResult | null> => {
    return invoke<OpenFileResult | null>('open_file_dialog');
  },

  openPdfDialog: async (): Promise<string | null> => {
    return invoke<string | null>('open_pdf_dialog');
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

  createFile: async (dirPath: string, fileName: string): Promise<string> => {
    return invoke<string>('create_file', { dirPath, fileName });
  },

  createDirectory: async (
    parentPath: string,
    dirName: string
  ): Promise<string> => {
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
  ): Promise<string> => {
    return invoke<string>('save_image', { baseDir, base64Data, mimeType });
  },

  readImageBase64: async (
    filePath: string,
    baseDir?: string
  ): Promise<string> => {
    return invoke<string>('read_image_base64', {
      filePath,
      baseDir: baseDir || null,
    });
  },

  showInFolder: async (filePath: string): Promise<void> => {
    return invoke<void>('show_in_folder', { filePath });
  },

  exportPDF: async (
    htmlContent?: string,
    defaultTitle?: string
  ): Promise<boolean> => {
    if (htmlContent) {
      let contentToPrint = htmlContent;
      if (defaultTitle && !/<title>/i.test(htmlContent)) {
        contentToPrint = htmlContent.replace(
          /<head([^>]*)>/i,
          `<head$1><title>${escapeHtml(defaultTitle)}</title>`
        );
      }

      const printFrame = document.createElement('iframe');
      printFrame.style.position = 'fixed';
      printFrame.style.right = '0';
      printFrame.style.bottom = '0';
      printFrame.style.width = '0';
      printFrame.style.height = '0';
      printFrame.style.border = '0';
      document.body.appendChild(printFrame);

      let printed = false;
      const cleanup = () => {
        if (printFrame.parentNode) {
          printFrame.parentNode.removeChild(printFrame);
        }
      };

      const doPrint = () => {
        if (printed) return;
        printed = true;
        const frameWindow = printFrame.contentWindow;
        if (frameWindow) {
          frameWindow.addEventListener('afterprint', cleanup);
          frameWindow.focus();
          frameWindow.print();
        }
      };

      printFrame.onload = () => {
        doPrint();
      };

      printFrame.srcdoc = contentToPrint;

      setTimeout(() => {
        doPrint();
      }, 500);

      setTimeout(cleanup, 2000);

      return true;
    }
    window.print();
    return true;
  },

  saveHTMLFile: async (
    htmlContent: string,
    defaultName?: string,
    defaultDir?: string | null
  ): Promise<string | null> => {
    return invoke<string | null>('save_html', {
      htmlContent,
      defaultName: defaultName || null,
      defaultDir: defaultDir || null,
    });
  },

  getAppVersion: async (): Promise<string> => {
    return '1.2.0';
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

  setWindowTheme: async (theme: 'light' | 'dark' | 'auto'): Promise<void> => {
    try {
      await invoke<void>('set_window_theme', { theme });
    } catch (err) {
      console.warn('Failed to set window theme:', err);
    }
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

  processPdfOCR: async (options: OCROptions): Promise<OCRResult> => {
    return invoke<OCRResult>('process_pdf_ocr', {
      pdfPath: options.pdfPath,
      pages: options.pages || null,
      languages: options.languages || null,
      mode: options.mode || null,
      extractImages: options.extractImages ?? true,
      outputDir: options.outputDir || null,
      subfolder: options.subfolder || null,
      imagePlacement: options.imagePlacement || null,
    });
  },

  onOCRProgress: (callback: (progress: OCRProgress) => void): (() => void) => {
    return subscribeEvent<OCRProgress>('ocr:progress', callback);
  },

  onMenuCommand: (callback: (command: string) => void): (() => void) => {
    return subscribeEvent<string>('menu:command', callback);
  },

  onWindowClosing: (callback: () => void): (() => void) => {
    return subscribeEvent<void>('app:window-closing', () => callback());
  },

  onFileDrop: (callback: (paths: string[]) => void): (() => void) => {
    let cancelled = false;
    let unlistenFn: (() => void) | null = null;

    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === 'drop' && event.payload.paths.length > 0) {
          callback(event.payload.paths);
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlistenFn = fn;
        }
      });

    return () => {
      cancelled = true;
      if (unlistenFn) {
        unlistenFn();
        unlistenFn = null;
      }
    };
  },

  historyGetSettings: async (): Promise<HistorySettings> => {
    return invoke<HistorySettings>('history_get_settings');
  },

  historyUpdateSettings: async (settings: HistorySettings): Promise<HistorySettings> => {
    return invoke<HistorySettings>('history_update_settings', { settings });
  },

  historyListSnapshots: async (filePath: string): Promise<SnapshotSummary[]> => {
    return invoke<SnapshotSummary[]>('history_list_snapshots', { filePath });
  },

  historyListSnapshotsByFileId: async (fileId: number): Promise<SnapshotSummary[]> => {
    return invoke<SnapshotSummary[]>('history_list_snapshots_by_file_id', { fileId });
  },

  historyGetSnapshotContent: async (snapshotId: number): Promise<string> => {
    return invoke<string>('history_get_snapshot_content', { snapshotId });
  },

  historyDiffSnapshot: async (
    snapshotId: number,
    currentContent: string
  ): Promise<DiffResult> => {
    return invoke<DiffResult>('history_diff_snapshot', { snapshotId, currentContent });
  },

  historyRestoreSnapshot: async (
    filePath: string,
    snapshotId: number,
    currentContent: string
  ): Promise<string> => {
    return invoke<string>('history_restore_snapshot', {
      filePath,
      snapshotId,
      currentContent,
    });
  },

  historyListDeletedFiles: async (): Promise<DeletedFileSummary[]> => {
    return invoke<DeletedFileSummary[]>('history_list_deleted_files');
  },

  historyRecoverFile: async (
    fileId: number,
    snapshotId: number,
    targetDir?: string | null
  ): Promise<RecoverFileResult> => {
    return invoke<RecoverFileResult>('history_recover_file', {
      fileId,
      snapshotId,
      targetDir: targetDir ?? null,
    });
  },

  historyDeleteFileHistory: async (fileId: number): Promise<boolean> => {
    return invoke<boolean>('history_delete_file_history', { fileId });
  },

  historyClearAll: async (): Promise<boolean> => {
    return invoke<boolean>('history_clear_all');
  },

  historyGetStats: async (): Promise<HistoryStats> => {
    return invoke<HistoryStats>('history_get_stats');
  },

  driveGetStatus: async (): Promise<DriveStatus> => {
    return invoke<DriveStatus>('drive_get_status');
  },

  driveUpdateSettings: async (settings: DriveSettings): Promise<DriveStatus> => {
    return invoke<DriveStatus>('drive_update_settings', { settings });
  },

  driveConnect: async (
    workspaceDir?: string | null,
    openFilePaths?: string[] | null
  ): Promise<DriveStatus> => {
    return invoke<DriveStatus>('drive_connect', {
      workspaceDir: workspaceDir ?? null,
      openFilePaths: openFilePaths ?? null,
    });
  },

  driveCancelAuth: async (): Promise<boolean> => {
    return invoke<boolean>('drive_cancel_auth');
  },

  driveDisconnect: async (): Promise<DriveStatus> => {
    return invoke<DriveStatus>('drive_disconnect');
  },

  driveSyncNow: async (
    workspaceDir?: string | null,
    openFilePaths?: string[] | null
  ): Promise<DriveStatus> => {
    return invoke<DriveStatus>('drive_sync_now', {
      workspaceDir: workspaceDir ?? null,
      openFilePaths: openFilePaths ?? null,
    });
  },

  driveFlushBeforeQuit: async (timeoutMs?: number | null): Promise<DriveFlushResult> => {
    return invoke<DriveFlushResult>('drive_flush_before_quit', {
      timeoutMs: timeoutMs ?? null,
    });
  },

  driveListRemoteFiles: async (): Promise<DriveRemoteFile[]> => {
    return invoke<DriveRemoteFile[]>('drive_list_remote_files');
  },

  driveDownloadFileContent: async (fileId: string): Promise<string> => {
    return invoke<string>('drive_download_file_content', { fileId });
  },

  driveRestoreFile: async (
    fileId: string,
    remotePath: string,
    targetDir: string,
    preserveFolders: boolean
  ): Promise<DriveRestoreResult> => {
    return invoke<DriveRestoreResult>('drive_restore_file', {
      fileId,
      remotePath,
      targetDir,
      preserveFolders,
    });
  },

  driveRestoreAll: async (
    targetDir: string,
    preserveFolders: boolean
  ): Promise<DriveBatchRestoreResult> => {
    return invoke<DriveBatchRestoreResult>('drive_restore_all', {
      targetDir,
      preserveFolders,
    });
  },

  onDriveStatusChanged: (callback: (status: DriveStatus) => void): (() => void) => {
    return subscribeEvent<DriveStatus>('drive:status-changed', callback);
  },
};

// Initialize bridge on window
if (typeof window !== 'undefined') {
  if (isTauri()) {
    (window as any).electronAPI = tauriAPI;
    (window as any).tauriAPI = tauriAPI;
  }
}
