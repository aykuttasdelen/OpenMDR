import React, { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import { TitleBar } from './components/TitleBar';
import { Sidebar } from './components/Sidebar';
import { TabBar } from './components/TabBar';
import { FormattingBar } from './components/FormattingBar';
import { Editor, EditorHandle } from './components/Editor';
import { Preview } from './components/Preview';
import { StatusBar } from './components/StatusBar';
import { CommandPalette, CommandItem } from './components/CommandPalette';
import { ExportModal } from './components/ExportModal';
import { SettingsModal } from './components/SettingsModal';
import { UnsavedChangesModal } from './components/UnsavedChangesModal';
import { Tab, FileItem, ViewMode, ThemeMode } from './types/electron';
import { DEFAULT_WELCOME_MARKDOWN } from './utils/defaultDocument';
import { useTranslation } from './i18n/LanguageContext';
import { 
  extractOutline, 
  calculateStats, 
  toggleTaskInMarkdown,
  getBasename,
  getDirname,
  isMarkdownFileName,
  getDefaultSaveFilePath,
  getUnsavedTabs
} from './utils/markdownUtils';
import { 
  FilePlus, 
  FolderOpen, 
  Save, 
  Sidebar as SidebarIcon, 
  Columns2, 
  FileEdit, 
  Eye, 
  Share2, 
  Sun, 
  Moon, 
  Search,
  ZoomIn,
  ZoomOut,
  RotateCcw,
  CheckCircle2,
  Copy,
  Settings as SettingsIcon,
  FolderCheck,
  X,
  XCircle
} from 'lucide-react';

const STORAGE_KEYS = {
  THEME: 'openmd-theme',
  TABS: 'openmd-session-tabs',
  ACTIVE_TAB: 'openmd-session-active-tab',
  ROOT_FOLDER: 'openmd-session-root-folder',
  AUTO_SAVE: 'openmd-auto-save',
  FONT_SIZE: 'openmd-font-size',
  SPLIT_RATIO: 'openmd-split-ratio',
  SIDEBAR_WIDTH: 'openmd-sidebar-width',
  DEFAULT_SAVE_PATH: 'openmd-default-save-path',
  AUTO_SAVE_TO_DEFAULT: 'openmd-auto-save-to-default',
  FILTER_MD_ONLY: 'openmd-filter-md-only',
  DISMISSED_PROMPT: 'openmd-dismissed-save-prompt'
};

export const App: React.FC = () => {
  const { t, language } = useTranslation();

  // Theme state
  const [theme, setTheme] = useState<ThemeMode>(() => {
    return (localStorage.getItem(STORAGE_KEYS.THEME) as ThemeMode) || 'dark';
  });

  // Tabs state with session restoration
  const [tabs, setTabs] = useState<Tab[]>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEYS.TABS);
      if (saved) {
        const parsed = JSON.parse(saved);
        if (Array.isArray(parsed) && parsed.length > 0) {
          return parsed;
        }
      }
    } catch (e) {
      console.warn('Failed to parse saved tabs session:', e);
    }
    return [
      {
        id: 'tab-welcome',
        title: 'Hoş Geldiniz.md',
        content: DEFAULT_WELCOME_MARKDOWN,
        savedContent: DEFAULT_WELCOME_MARKDOWN,
        isModified: false
      }
    ];
  });

  const [activeTabId, setActiveTabId] = useState<string>(() => {
    const savedActive = localStorage.getItem(STORAGE_KEYS.ACTIVE_TAB);
    if (savedActive && tabs.some(t => t.id === savedActive)) {
      return savedActive;
    }
    return tabs[0]?.id || 'tab-welcome';
  });

  // View state
  const [viewMode, setViewMode] = useState<ViewMode>('split');
  const [sidebarOpen, setSidebarOpen] = useState(true);
  const [commandPaletteOpen, setCommandPaletteOpen] = useState(false);
  const [exportModalOpen, setExportModalOpen] = useState(false);
  const [settingsModalOpen, setSettingsModalOpen] = useState(false);
  const [unsavedModalOpen, setUnsavedModalOpen] = useState(false);
  const [pendingCloseAction, setPendingCloseAction] = useState<'window' | 'workspace' | null>(null);

  // Settings & Storage Options
  const [defaultSavePath, setDefaultSavePath] = useState<string | null>(() => {
    return localStorage.getItem(STORAGE_KEYS.DEFAULT_SAVE_PATH) || null;
  });
  const [autoSaveToDefault, setAutoSaveToDefault] = useState<boolean>(() => {
    const saved = localStorage.getItem(STORAGE_KEYS.AUTO_SAVE_TO_DEFAULT);
    return saved !== null ? saved === 'true' : true;
  });
  const [filterMdOnly, setFilterMdOnly] = useState<boolean>(() => {
    const saved = localStorage.getItem(STORAGE_KEYS.FILTER_MD_ONLY);
    return saved !== null ? saved === 'true' : true;
  });
  const [dismissedPrompt, setDismissedPrompt] = useState<boolean>(() => {
    return localStorage.getItem(STORAGE_KEYS.DISMISSED_PROMPT) === 'true';
  });

  // Layout sizing state (Resizable Sidebar & Split Pane)
  const [splitRatio, setSplitRatio] = useState<number>(() => {
    const saved = localStorage.getItem(STORAGE_KEYS.SPLIT_RATIO);
    return saved ? parseFloat(saved) : 50;
  });
  const [sidebarWidth, setSidebarWidth] = useState<number>(() => {
    const saved = localStorage.getItem(STORAGE_KEYS.SIDEBAR_WIDTH);
    return saved ? parseInt(saved, 10) : 260;
  });
  const [isDraggingSplit, setIsDraggingSplit] = useState(false);
  const [isDraggingSidebar, setIsDraggingSidebar] = useState(false);
  const splitPaneRef = useRef<HTMLDivElement>(null);

  // Auto-Save & Font Size Zoom
  const [autoSaveEnabled, setAutoSaveEnabled] = useState<boolean>(() => {
    return localStorage.getItem(STORAGE_KEYS.AUTO_SAVE) === 'true';
  });
  const [fontSize, setFontSize] = useState<number>(() => {
    const saved = localStorage.getItem(STORAGE_KEYS.FONT_SIZE);
    return saved ? parseInt(saved, 10) : 14;
  });

  // File tree state
  const [rootFolderPath, setRootFolderPath] = useState<string | undefined>(() => {
    return localStorage.getItem(STORAGE_KEYS.ROOT_FOLDER) || undefined;
  });
  const [fileTree, setFileTree] = useState<FileItem[]>([]);

  // Editor cursor & scroll
  const [cursorPos, setCursorPos] = useState({ line: 1, col: 1 });
  const editorRef = useRef<EditorHandle>(null);
  const previewScrollRef = useRef<HTMLDivElement>(null);

  // Active tab helper
  const activeTab = useMemo(() => {
    return tabs.find(t => t.id === activeTabId) || tabs[0];
  }, [tabs, activeTabId]);

  const baseDir = useMemo(() => {
    if (activeTab?.filePath) {
      return getDirname(activeTab.filePath);
    }
    return rootFolderPath || defaultSavePath || undefined;
  }, [activeTab?.filePath, rootFolderPath, defaultSavePath]);

  // Document outline and stats
  const outlineItems = useMemo(() => {
    return extractOutline(activeTab ? activeTab.content : '');
  }, [activeTab]);

  const stats = useMemo(() => {
    return calculateStats(activeTab ? activeTab.content : '');
  }, [activeTab]);

  // Unsaved tabs list
  const unsavedTabs = useMemo(() => {
    return getUnsavedTabs(tabs);
  }, [tabs]);

  // Apply theme to document element
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    localStorage.setItem(STORAGE_KEYS.THEME, theme);
  }, [theme]);

  // Session persistence synchronization
  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEYS.TABS, JSON.stringify(tabs));
    } catch (e) {
      console.warn('Failed to save tabs session:', e);
    }
  }, [tabs]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.ACTIVE_TAB, activeTabId);
  }, [activeTabId]);

  useEffect(() => {
    if (rootFolderPath) {
      localStorage.setItem(STORAGE_KEYS.ROOT_FOLDER, rootFolderPath);
    } else {
      localStorage.removeItem(STORAGE_KEYS.ROOT_FOLDER);
    }
  }, [rootFolderPath]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.AUTO_SAVE, String(autoSaveEnabled));
  }, [autoSaveEnabled]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.FONT_SIZE, String(fontSize));
  }, [fontSize]);

  // Initialize default save path to Documents if not already configured
  useEffect(() => {
    const initDefaultDocumentsPath = async () => {
      const saved = localStorage.getItem(STORAGE_KEYS.DEFAULT_SAVE_PATH);
      if (!saved && window.electronAPI) {
        try {
          const docFn = window.electronAPI.getDefaultDocumentsDir || window.electronAPI.getDocumentsDir;
          if (docFn) {
            const dir = await docFn();
            if (dir) {
              setDefaultSavePath(dir);
              localStorage.setItem(STORAGE_KEYS.DEFAULT_SAVE_PATH, dir);
            }
          }
        } catch (err) {
          console.warn('Failed to retrieve default documents directory:', err);
        }
      }
    };
    initDefaultDocumentsPath();
  }, []);

  useEffect(() => {
    if (defaultSavePath) {
      localStorage.setItem(STORAGE_KEYS.DEFAULT_SAVE_PATH, defaultSavePath);
    } else {
      localStorage.removeItem(STORAGE_KEYS.DEFAULT_SAVE_PATH);
    }
  }, [defaultSavePath]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.AUTO_SAVE_TO_DEFAULT, String(autoSaveToDefault));
  }, [autoSaveToDefault]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.FILTER_MD_ONLY, String(filterMdOnly));
  }, [filterMdOnly]);

  useEffect(() => {
    localStorage.setItem(STORAGE_KEYS.DISMISSED_PROMPT, String(dismissedPrompt));
  }, [dismissedPrompt]);

  // Restore workspace directory on startup
  useEffect(() => {
    if (rootFolderPath && window.electronAPI) {
      window.electronAPI.readDirectory(rootFolderPath, filterMdOnly)
        .then(tree => setFileTree(tree))
        .catch(err => {
          console.warn('Failed to restore workspace folder:', err);
          setRootFolderPath(undefined);
        });
    }
  }, [filterMdOnly]);

  // Content change handler with docId support
  const handleContentChange = useCallback((newContent: string, docId?: string) => {
    const targetId = docId || activeTabId;
    setTabs(prev => prev.map(t => {
      if (t.id === targetId) {
        return {
          ...t,
          content: newContent,
          isModified: newContent !== t.savedContent
        };
      }
      return t;
    }));
  }, [activeTabId]);

  const handleRefreshTree = useCallback(async () => {
    if (rootFolderPath && window.electronAPI) {
      const tree = await window.electronAPI.readDirectory(rootFolderPath, filterMdOnly);
      setFileTree(tree);
    }
  }, [rootFolderPath, filterMdOnly]);

  // Auto-Save effect supporting both existing files and untitled documents with default save location
  useEffect(() => {
    if (!autoSaveEnabled || !activeTab?.isModified || !window.electronAPI) {
      return;
    }

    // If active tab has no file path yet, check if defaultSavePath is configured
    if (!activeTab.filePath && (!defaultSavePath || !autoSaveToDefault)) {
      return;
    }

    const targetTabId = activeTab.id;
    const targetFilePath = activeTab.filePath;
    const targetTitle = activeTab.title;
    const contentToWrite = activeTab.content;

    const timer = setTimeout(async () => {
      try {
        if (!window.electronAPI) return;
        let finalPath = targetFilePath;
        let finalTitle = targetTitle;
        if (!finalPath && defaultSavePath) {
          finalPath = await window.electronAPI.getAvailablePath(defaultSavePath, targetTitle);
          finalTitle = getBasename(finalPath);
        }

        if (finalPath) {
          await window.electronAPI.writeFile(finalPath, contentToWrite);
          setTabs(prev => prev.map(t => {
            if (t.id === targetTabId) {
              return {
                ...t,
                title: finalTitle,
                filePath: finalPath,
                savedContent: contentToWrite,
                isModified: t.content !== contentToWrite
              };
            }
            return t;
          }));

          if (rootFolderPath) {
            handleRefreshTree();
          }
        }
      } catch (err) {
        console.error('Auto-save error:', err);
      }
    }, 1000);

    return () => clearTimeout(timer);
  }, [
    autoSaveEnabled, 
    autoSaveToDefault, 
    defaultSavePath, 
    activeTab?.content, 
    activeTab?.filePath, 
    activeTab?.isModified, 
    activeTab?.id, 
    activeTab?.title, 
    rootFolderPath, 
    handleRefreshTree
  ]);

  // Synchronized scroll from editor to preview
  const handleEditorScroll = (scrollPercentage: number) => {
    if (previewScrollRef.current) {
      const target = previewScrollRef.current;
      const maxScroll = target.scrollHeight - target.clientHeight;
      if (maxScroll > 0) {
        target.scrollTop = scrollPercentage * maxScroll;
      }
    }
  };

  // Safe new file creation with smart numbering defaulting to Documents
  const handleNewFile = useCallback(async () => {
    const newId = `tab-${Date.now()}`;
    const untitledPrefix = t('app.untitled');
    const untitledRegex = new RegExp(`^${untitledPrefix}(?: (\\d+))?\\.md$`);
    let maxNum = 0;
    tabs.forEach(t => {
      const match = t.title.match(untitledRegex);
      if (match) {
        const num = match[1] ? parseInt(match[1], 10) : 1;
        if (num > maxNum) maxNum = num;
      }
    });

    const newTitle = maxNum === 0 ? `${untitledPrefix}.md` : `${untitledPrefix} ${maxNum + 1}.md`;

    // Resolve target path under defaultSavePath (Documents) if available
    let initialFilePath: string | undefined = undefined;
    if (defaultSavePath && window.electronAPI?.getAvailablePath) {
      try {
        initialFilePath = await window.electronAPI.getAvailablePath(defaultSavePath, newTitle);
      } catch (e) {
        console.warn('Failed to resolve initial file path for new file:', e);
      }
    }

    const newTab: Tab = {
      id: newId,
      title: newTitle,
      filePath: initialFilePath,
      content: `# ${newTitle.replace(/\.md$/, '')}\n\n`,
      savedContent: '',
      isModified: false
    };

    setTabs(prev => [...prev, newTab]);
    setActiveTabId(newId);
    setTimeout(() => editorRef.current?.focus(), 60);
  }, [tabs, defaultSavePath, t]);

  const openFileInTab = useCallback((filePath: string, name: string, content: string) => {
    const existing = tabs.find(t => t.filePath === filePath);
    if (existing) {
      setActiveTabId(existing.id);
      setTimeout(() => editorRef.current?.focus(), 60);
      return;
    }

    const newTab: Tab = {
      id: `tab-${Date.now()}`,
      title: name,
      filePath,
      content,
      savedContent: content,
      isModified: false
    };

    setTabs(prev => [...prev, newTab]);
    setActiveTabId(newTab.id);
    setTimeout(() => editorRef.current?.focus(), 60);
  }, [tabs]);

  const handleOpenFolder = useCallback(async () => {
    if (window.electronAPI) {
      try {
        const result = await window.electronAPI.openDirectoryDialog(filterMdOnly);
        if (result) {
          setRootFolderPath(result.dirPath);
          setFileTree(result.tree);
          setSidebarOpen(true);
        }
      } catch (err: any) {
        console.error('Folder open error:', err);
        alert(t('app.folderOpenError', { error: err.message }));
      }
    } else {
      alert(t('app.electronOnly'));
    }
  }, [filterMdOnly, t]);

  const performCloseWorkspace = useCallback(() => {
    const currentRoot = rootFolderPath;
    setRootFolderPath(undefined);
    setFileTree([]);

    setTabs(prev => {
      const remaining = currentRoot 
        ? prev.filter(t => !t.filePath || (!t.filePath.startsWith(currentRoot + '/') && !t.filePath.startsWith(currentRoot + '\\')))
        : [];
      
      if (remaining.length === 0) {
        const untitledPrefix = t('app.untitled');
        const newTitle = `${untitledPrefix}.md`;
        const newId = `tab-${Date.now()}`;
        setActiveTabId(newId);
        return [{
          id: newId,
          title: newTitle,
          content: `# ${untitledPrefix}\n\n`,
          savedContent: '',
          isModified: false
        }];
      }
      setActiveTabId(remaining[0].id);
      return remaining;
    });
  }, [rootFolderPath, t]);

  // Close workspace folder with unsaved changes verification (Task 2)
  const handleCloseWorkspace = useCallback(() => {
    const workspaceTabs = rootFolderPath 
      ? tabs.filter(t => t.filePath && (t.filePath.startsWith(rootFolderPath + '/') || t.filePath.startsWith(rootFolderPath + '\\')))
      : tabs;
    const hasUnsavedInWorkspace = workspaceTabs.some(t => t.isModified);
    if (hasUnsavedInWorkspace) {
      setPendingCloseAction('workspace');
      setUnsavedModalOpen(true);
    } else {
      performCloseWorkspace();
    }
  }, [tabs, rootFolderPath, performCloseWorkspace]);

  const handleOpenFile = useCallback(async () => {
    if (window.electronAPI) {
      try {
        const result = await window.electronAPI.openFileDialog();
        if (result) {
          openFileInTab(result.filePath, result.name, result.content);
        }
      } catch (err: any) {
        console.error('File open error:', err);
        alert(t('app.fileOpenError', { error: err.message }));
      }
    } else {
      const input = document.createElement('input');
      input.type = 'file';
      input.accept = '.md,.mdx,.markdown,.txt';
      input.onchange = async (e: any) => {
        const file = e.target.files?.[0];
        if (file) {
          const content = await file.text();
          openFileInTab(file.name, file.name, content);
        }
      };
      input.click();
    }
  }, [openFileInTab, t]);

  const handleSaveFile = useCallback(async (saveAs = false) => {
    if (!activeTab) return;

    if (window.electronAPI) {
      try {
        let targetPath = activeTab.filePath;
        const contentToWrite = activeTab.content;

        if (!targetPath || saveAs) {
          const defaultDir = rootFolderPath || defaultSavePath || undefined;
          const selectedPath = await window.electronAPI.saveFileDialog(activeTab.title, contentToWrite, defaultDir);
          if (!selectedPath) return;
          targetPath = selectedPath;
        } else {
          await window.electronAPI.writeFile(targetPath, contentToWrite);
        }

        const fileName = getBasename(targetPath) || activeTab.title;
        setTabs(prev => prev.map(t => {
          if (t.id === activeTab.id) {
            return {
              ...t,
              title: fileName,
              filePath: targetPath,
              savedContent: contentToWrite,
              isModified: t.content !== contentToWrite
            };
          }
          return t;
        }));

        if (rootFolderPath) {
          handleRefreshTree();
        }
      } catch (err: any) {
        alert(t('app.fileSaveError', { error: err.message }));
      }
    } else {
      const blob = new Blob([activeTab.content], { type: 'text/markdown;charset=utf-8' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = activeTab.title;
      a.click();
      URL.revokeObjectURL(url);
      setTabs(prev => prev.map(t => t.id === activeTab.id ? { ...t, isModified: false, savedContent: t.content } : t));
    }
  }, [activeTab, defaultSavePath, rootFolderPath, handleRefreshTree, t]);

  // Safe tab closing with unsaved warning
  const handleCloseTab = useCallback((id: string) => {
    const tabToClose = tabs.find(t => t.id === id);
    if (tabToClose?.isModified) {
      const confirmClose = window.confirm(t('app.confirmCloseModified', { title: tabToClose.title }));
      if (!confirmClose) return;
    }

    if (tabs.length <= 1) {
      const newId = `tab-${Date.now()}`;
      const newTab: Tab = {
        id: newId,
        title: `${t('app.untitled')}.md`,
        content: `# ${t('app.untitled')}\n\n`,
        savedContent: '',
        isModified: false
      };
      setTabs([newTab]);
      setActiveTabId(newId);
      setTimeout(() => editorRef.current?.focus(), 60);
      return;
    }

    const tabIndex = tabs.findIndex(t => t.id === id);
    const remaining = tabs.filter(t => t.id !== id);
    setTabs(remaining);

    if (activeTabId === id) {
      const newIndex = Math.max(0, Math.min(tabIndex, remaining.length - 1));
      setActiveTabId(remaining[newIndex].id);
      setTimeout(() => editorRef.current?.focus(), 60);
    }
  }, [tabs, activeTabId, t]);

  // Unsaved Changes Modal Actions (Task 2)
  const handleSaveAllAndClose = async () => {
    if (!window.electronAPI) return;
    const modifiedTabs = tabs.filter(t => t.isModified);

    for (const tab of modifiedTabs) {
      let targetPath = tab.filePath;
      if (!targetPath) {
        if (defaultSavePath) {
          targetPath = await window.electronAPI.getAvailablePath(defaultSavePath, tab.title);
        } else {
          const selected = await window.electronAPI.saveFileDialog(tab.title, tab.content, rootFolderPath || undefined);
          if (!selected) {
            // User canceled save dialog! Abort to avoid losing work!
            return;
          }
          targetPath = selected;
        }
      }

      if (targetPath) {
        try {
          await window.electronAPI.writeFile(targetPath, tab.content);
          const savedPath = targetPath;
          setTabs(prev => prev.map(t => t.id === tab.id ? {
            ...t,
            title: getBasename(savedPath),
            filePath: savedPath,
            savedContent: tab.content,
            isModified: false
          } : t));
        } catch (err: any) {
          alert(t('app.fileSaveError', { error: err.message }));
          return;
        }
      }
    }

    setUnsavedModalOpen(false);

    if (pendingCloseAction === 'window') {
      window.electronAPI.closeWindow(true);
    } else if (pendingCloseAction === 'workspace') {
      performCloseWorkspace();
    }
    setPendingCloseAction(null);
  };

  const handleDiscardAndClose = () => {
    setUnsavedModalOpen(false);
    if (pendingCloseAction === 'window') {
      window.electronAPI?.closeWindow(true);
    } else if (pendingCloseAction === 'workspace') {
      performCloseWorkspace();
    }
    setPendingCloseAction(null);
  };

  const handleCancelUnsavedModal = () => {
    setUnsavedModalOpen(false);
    setPendingCloseAction(null);
    if (window.electronAPI) {
      window.electronAPI.cancelClose();
    }
  };

  // Default Save Location Selection (Task 3)
  const handleSelectDefaultSavePath = async () => {
    if (!window.electronAPI) return;
    const result = await window.electronAPI.openDirectoryDialog();
    if (result) {
      setDefaultSavePath(result.dirPath);
      setAutoSaveEnabled(true);
      setAutoSaveToDefault(true);
      setDismissedPrompt(true);
    }
  };

  const handleClearDefaultSavePath = () => {
    setDefaultSavePath(null);
  };

  const handleSetDocumentsDefault = async () => {
    if (!window.electronAPI) return;
    try {
      const docFn = window.electronAPI.getDefaultDocumentsDir || window.electronAPI.getDocumentsDir;
      if (docFn) {
        const dir = await docFn();
        if (dir) {
          setDefaultSavePath(dir);
          setAutoSaveEnabled(true);
          setAutoSaveToDefault(true);
          setDismissedPrompt(true);
        }
      }
    } catch (e) {
      console.warn('Failed to set documents as default:', e);
    }
  };

  const handleCreateFileInFolder = async (dirPath: string, fileName: string) => {
    if (!window.electronAPI) return;
    const hasExt = /\.(md|mdx)$/i.test(fileName);
    const finalName = hasExt ? fileName : `${fileName}.md`;
    const createdPath = await window.electronAPI.createFile(dirPath, finalName);
    if (createdPath) {
      await handleRefreshTree();
      const content = await window.electronAPI.readFile(createdPath);
      openFileInTab(createdPath, getBasename(createdPath), content);
    }
  };

  const handleCreateFolderInFolder = async (parentPath: string, folderName: string) => {
    if (!window.electronAPI) return;
    await window.electronAPI.createDirectory(parentPath, folderName);
    await handleRefreshTree();
  };

  const handleRenameFile = async (oldPath: string, newPath: string) => {
    if (!window.electronAPI) return;
    await window.electronAPI.renameFile(oldPath, newPath);
    await handleRefreshTree();

    const newName = getBasename(newPath);
    setTabs(prev => prev.map(t => {
      if (!t.filePath) return t;
      if (t.filePath === oldPath) {
        return {
          ...t,
          title: newName,
          filePath: newPath
        };
      }
      if (t.filePath.startsWith(oldPath + '/') || t.filePath.startsWith(oldPath + '\\')) {
        const updatedFilePath = newPath + t.filePath.slice(oldPath.length);
        return {
          ...t,
          filePath: updatedFilePath
        };
      }
      return t;
    }));

    if (rootFolderPath === oldPath) {
      setRootFolderPath(newPath);
    }
  };

  const handleDeleteFileFromTree = async (filePath: string) => {
    if (!window.electronAPI) return;
    try {
      await window.electronAPI.deleteFile(filePath);
      await handleRefreshTree();
      const tabsToClose = tabs.filter(t => t.filePath && (
        t.filePath === filePath ||
        t.filePath.startsWith(filePath + '/') ||
        t.filePath.startsWith(filePath + '\\')
      ));
      for (const t of tabsToClose) {
        handleCloseTab(t.id);
      }
      if (rootFolderPath === filePath) {
        setRootFolderPath(undefined);
      }
    } catch (err: any) {
      alert(t('sidebar.deleteError', { error: err.message }));
    }
  };

  const handleShowInFinder = async (filePath: string) => {
    if (window.electronAPI) {
      await window.electronAPI.showInFolder(filePath);
    }
  };

  const handleFileTreeItemClick = async (filePath: string, name: string) => {
    if (window.electronAPI) {
      try {
        const content = await window.electronAPI.readFile(filePath);
        openFileInTab(filePath, name, content);
      } catch (err: any) {
        alert(err.message);
      }
    }
  };

  // Interactive task checkbox toggle handler
  const handleToggleTask = (taskIndex: number, checked: boolean) => {
    if (!activeTab) return;
    const updatedContent = toggleTaskInMarkdown(activeTab.content, taskIndex, checked);
    handleContentChange(updatedContent, activeTab.id);
  };

  // Formatting insert helper
  const handleInsertFormat = useCallback((prefix: string, suffix?: string, defaultText?: string) => {
    editorRef.current?.insertText(prefix, suffix, defaultText);
  }, []);

  // Jump to heading line
  const handleSelectHeading = (line: number) => {
    editorRef.current?.scrollToLine(line);
  };

  // Font size zoom controls
  const handleZoomIn = () => setFontSize(prev => Math.min(28, prev + 1));
  const handleZoomOut = () => setFontSize(prev => Math.max(11, prev - 1));
  const handleResetZoom = () => setFontSize(14);

  // App Close Interception (Task 2)
  useEffect(() => {
    if (!window.electronAPI) return;

    const cleanup = window.electronAPI.onWindowClosing(() => {
      const hasUnsaved = tabs.some(t => t.isModified);
      if (hasUnsaved) {
        setPendingCloseAction('window');
        setUnsavedModalOpen(true);
      } else {
        window.electronAPI?.closeWindow(true);
      }
    });

    return cleanup;
  }, [tabs]);

  // Web fallback beforeunload warning
  useEffect(() => {
    const handleBeforeUnload = (e: BeforeUnloadEvent) => {
      if (tabs.some(t => t.isModified)) {
        e.preventDefault();
        e.returnValue = '';
      }
    };
    window.addEventListener('beforeunload', handleBeforeUnload);
    return () => window.removeEventListener('beforeunload', handleBeforeUnload);
  }, [tabs]);

  // Keyboard Shortcuts
  useEffect(() => {
    const isElectron = Boolean(window.electronAPI);

    const handleKeyDown = (e: KeyboardEvent) => {
      const isCmd = e.metaKey || e.ctrlKey;

      // Settings shortcut Cmd+,
      if (isCmd && e.key === ',') {
        e.preventDefault();
        setSettingsModalOpen(prev => !prev);
        return;
      }

      // Zoom shortcuts
      if (isCmd && (e.key === '=' || e.key === '+')) {
        e.preventDefault();
        handleZoomIn();
        return;
      }
      if (isCmd && e.key === '-') {
        e.preventDefault();
        handleZoomOut();
        return;
      }
      if (isCmd && e.key === '0') {
        e.preventDefault();
        handleResetZoom();
        return;
      }

      if (isElectron) return;

      // Web Fallback Shortcuts
      if (isCmd && e.key.toLowerCase() === 'k') {
        e.preventDefault();
        if (e.shiftKey) {
          handleInsertFormat('[', '](https://example.com)', 'link');
        } else {
          setCommandPaletteOpen(prev => !prev);
        }
      } else if (isCmd && e.key.toLowerCase() === 'n') {
        e.preventDefault();
        handleNewFile();
      } else if (isCmd && e.key.toLowerCase() === 'o') {
        e.preventDefault();
        if (e.shiftKey) {
          handleOpenFolder();
        } else {
          handleOpenFile();
        }
      } else if (isCmd && e.key.toLowerCase() === 's') {
        e.preventDefault();
        handleSaveFile(e.shiftKey);
      } else if (isCmd && e.key.toLowerCase() === 'w') {
        e.preventDefault();
        handleCloseTab(activeTabId);
      } else if (isCmd && e.key === '\\') {
        e.preventDefault();
        setSidebarOpen(prev => !prev);
      } else if (isCmd && e.key === '1') {
        e.preventDefault();
        setViewMode('editor');
      } else if (isCmd && e.key === '2') {
        e.preventDefault();
        setViewMode('split');
      } else if (isCmd && e.key === '3') {
        e.preventDefault();
        setViewMode('preview');
      } else if (isCmd && e.key.toLowerCase() === 'f') {
        e.preventDefault();
        editorRef.current?.openFind();
      } else if (isCmd && e.key.toLowerCase() === 'b') {
        e.preventDefault();
        handleInsertFormat('**', '**', 'bold');
      } else if (isCmd && e.key.toLowerCase() === 'i') {
        e.preventDefault();
        handleInsertFormat('*', '*', 'italic');
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [
    activeTabId, 
    handleNewFile, 
    handleOpenFile, 
    handleOpenFolder, 
    handleSaveFile, 
    handleCloseTab,
    handleInsertFormat
  ]);

  // Electron Native Menu Command Dispatcher
  useEffect(() => {
    if (!window.electronAPI) return;

    const cleanup = window.electronAPI.onMenuCommand((cmd) => {
      switch (cmd) {
        case 'new-file': handleNewFile(); break;
        case 'open-file': handleOpenFile(); break;
        case 'open-folder': handleOpenFolder(); break;
        case 'close-workspace': handleCloseWorkspace(); break;
        case 'save-file': handleSaveFile(false); break;
        case 'save-file-as': handleSaveFile(true); break;
        case 'close-tab': handleCloseTab(activeTabId); break;
        case 'toggle-sidebar': setSidebarOpen(prev => !prev); break;
        case 'view-editor': setViewMode('editor'); break;
        case 'view-split': setViewMode('split'); break;
        case 'view-preview': setViewMode('preview'); break;
        case 'command-palette': setCommandPaletteOpen(true); break;
        case 'open-settings': setSettingsModalOpen(true); break;
        case 'export-html': setExportModalOpen(true); break;
        case 'export-pdf': setExportModalOpen(true); break;
        case 'find': editorRef.current?.openFind(); break;
        case 'format-bold': handleInsertFormat('**', '**', 'bold text'); break;
        case 'format-italic': handleInsertFormat('*', '*', 'italic text'); break;
        case 'format-code': handleInsertFormat('```\n', '\n```', '// code'); break;
        case 'format-math': handleInsertFormat('$$\n', '\n$$', 'E = mc^2'); break;
        case 'format-link': handleInsertFormat('[', '](https://example.com)', 'link'); break;
      }
    });

    return cleanup;
  }, [
    activeTabId, 
    handleNewFile, 
    handleOpenFile, 
    handleOpenFolder, 
    handleCloseWorkspace,
    handleSaveFile, 
    handleCloseTab,
    handleInsertFormat
  ]);

  // Drag and Drop File & Folder Support
  useEffect(() => {
    const handleDragOver = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
    };

    const handleDrop = async (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const files = e.dataTransfer?.files;
      if (!files || files.length === 0) return;

      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        const filePath = (file as any).path || '';

        // If dropping a folder in Electron
        if (filePath && window.electronAPI) {
          try {
            const tree = await window.electronAPI.readDirectory(filePath, filterMdOnly);
            if (Array.isArray(tree)) {
              setRootFolderPath(filePath);
              setFileTree(tree);
              setSidebarOpen(true);
              continue;
            }
          } catch {
            // Not a directory
          }
        }

        if (isMarkdownFileName(file.name)) {
          const content = await file.text();
          openFileInTab(filePath || file.name, file.name, content);
        }
      }
    };

    window.addEventListener('dragover', handleDragOver);
    window.addEventListener('drop', handleDrop);
    return () => {
      window.removeEventListener('dragover', handleDragOver);
      window.removeEventListener('drop', handleDrop);
    };
  }, [openFileInTab, filterMdOnly]);

  // Keep refs for smooth mouseup localStorage persistence
  const splitRatioRef = useRef(splitRatio);
  useEffect(() => {
    splitRatioRef.current = splitRatio;
  }, [splitRatio]);

  const sidebarWidthRef = useRef(sidebarWidth);
  useEffect(() => {
    sidebarWidthRef.current = sidebarWidth;
  }, [sidebarWidth]);

  // Split Pane Resizing Handlers with requestAnimationFrame & is-resizing guard
  const handleSplitMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsDraggingSplit(true);
    document.body.classList.add('is-resizing');
  };

  useEffect(() => {
    if (!isDraggingSplit) return;

    let rafId: number | null = null;

    const handleMouseMove = (e: MouseEvent) => {
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        if (!splitPaneRef.current) return;
        const rect = splitPaneRef.current.getBoundingClientRect();
        const offset = e.clientX - rect.left;
        const newRatio = Math.max(20, Math.min(80, (offset / rect.width) * 100));
        setSplitRatio(newRatio);
      });
    };

    const handleMouseUp = () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      setIsDraggingSplit(false);
      document.body.classList.remove('is-resizing');
      localStorage.setItem(STORAGE_KEYS.SPLIT_RATIO, String(splitRatioRef.current));
    };

    window.addEventListener('mousemove', handleMouseMove, { passive: true });
    window.addEventListener('mouseup', handleMouseUp);
    return () => {
      if (rafId !== null) cancelAnimationFrame(rafId);
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
      document.body.classList.remove('is-resizing');
    };
  }, [isDraggingSplit]);

  // Sidebar Resizing Handlers with requestAnimationFrame & is-resizing guard
  const handleSidebarMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    setIsDraggingSidebar(true);
    document.body.classList.add('is-resizing');
  };

  useEffect(() => {
    if (!isDraggingSidebar) return;

    let rafId: number | null = null;

    const handleMouseMove = (e: MouseEvent) => {
      if (rafId !== null) return;
      rafId = requestAnimationFrame(() => {
        rafId = null;
        const newWidth = Math.max(180, Math.min(500, e.clientX));
        setSidebarWidth(newWidth);
      });
    };

    const handleMouseUp = () => {
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      setIsDraggingSidebar(false);
      document.body.classList.remove('is-resizing');
      localStorage.setItem(STORAGE_KEYS.SIDEBAR_WIDTH, String(sidebarWidthRef.current));
    };

    window.addEventListener('mousemove', handleMouseMove, { passive: true });
    window.addEventListener('mouseup', handleMouseUp);
    return () => {
      if (rafId !== null) cancelAnimationFrame(rafId);
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
      document.body.classList.remove('is-resizing');
    };
  }, [isDraggingSidebar]);

  // Command Palette Items (Localized with dynamic t)
  const commands: CommandItem[] = useMemo(() => [
    {
      id: 'cmd-new',
      title: t('command.newDoc'),
      shortcut: 'Cmd+N',
      icon: <FilePlus size={16} />,
      action: handleNewFile
    },
    {
      id: 'cmd-open',
      title: t('command.openDoc'),
      shortcut: 'Cmd+O',
      icon: <FolderOpen size={16} />,
      action: handleOpenFile
    },
    {
      id: 'cmd-open-folder',
      title: t('command.openFolder'),
      shortcut: 'Cmd+Shift+O',
      icon: <FolderOpen size={16} />,
      action: handleOpenFolder
    },
    {
      id: 'cmd-close-workspace',
      title: t('command.closeWorkspace'),
      icon: <XCircle size={16} />,
      action: handleCloseWorkspace
    },
    {
      id: 'cmd-save',
      title: t('command.saveDoc'),
      shortcut: 'Cmd+S',
      icon: <Save size={16} />,
      action: () => handleSaveFile(false)
    },
    {
      id: 'cmd-save-as',
      title: t('command.saveDocAs'),
      shortcut: 'Cmd+Shift+S',
      icon: <Save size={16} />,
      action: () => handleSaveFile(true)
    },
    {
      id: 'cmd-auto-save',
      title: t('command.autoSave', { status: autoSaveEnabled ? t('status.autoSaveOff') : t('status.autoSaveOn') }),
      icon: <CheckCircle2 size={16} />,
      action: () => setAutoSaveEnabled(prev => !prev)
    },
    {
      id: 'cmd-settings',
      title: t('command.settings'),
      shortcut: 'Cmd+,',
      icon: <SettingsIcon size={16} />,
      action: () => setSettingsModalOpen(true)
    },
    {
      id: 'cmd-export',
      title: t('command.export'),
      icon: <Share2 size={16} />,
      action: () => setExportModalOpen(true)
    },
    {
      id: 'cmd-copy-md',
      title: t('command.copyMarkdown'),
      icon: <Copy size={16} />,
      action: () => {
        if (activeTab) navigator.clipboard.writeText(activeTab.content);
      }
    },
    {
      id: 'cmd-sidebar',
      title: t('command.sidebar'),
      shortcut: 'Cmd+\\',
      icon: <SidebarIcon size={16} />,
      action: () => setSidebarOpen(prev => !prev)
    },
    {
      id: 'cmd-editor-view',
      title: t('command.editorOnly'),
      shortcut: 'Cmd+1',
      icon: <FileEdit size={16} />,
      action: () => setViewMode('editor')
    },
    {
      id: 'cmd-split-view',
      title: t('command.splitView'),
      shortcut: 'Cmd+2',
      icon: <Columns2 size={16} />,
      action: () => setViewMode('split')
    },
    {
      id: 'cmd-preview-view',
      title: t('command.previewOnly'),
      shortcut: 'Cmd+3',
      icon: <Eye size={16} />,
      action: () => setViewMode('preview')
    },
    {
      id: 'cmd-find',
      title: t('command.find'),
      shortcut: 'Cmd+F',
      icon: <Search size={16} />,
      action: () => editorRef.current?.openFind()
    },
    {
      id: 'cmd-zoom-in',
      title: t('command.zoomIn'),
      shortcut: 'Cmd++',
      icon: <ZoomIn size={16} />,
      action: handleZoomIn
    },
    {
      id: 'cmd-zoom-out',
      title: t('command.zoomOut'),
      shortcut: 'Cmd+-',
      icon: <ZoomOut size={16} />,
      action: handleZoomOut
    },
    {
      id: 'cmd-zoom-reset',
      title: t('command.zoomReset'),
      shortcut: 'Cmd+0',
      icon: <RotateCcw size={16} />,
      action: handleResetZoom
    },
    {
      id: 'cmd-theme',
      title: t('command.themeToggle', { target: theme === 'dark' ? t('settings.themeLight') : t('settings.themeDark') }),
      icon: theme === 'dark' ? <Sun size={16} /> : <Moon size={16} />,
      action: () => setTheme(prev => prev === 'dark' ? 'light' : 'dark')
    }
  ], [
    t, 
    handleNewFile, 
    handleOpenFile, 
    handleOpenFolder, 
    handleCloseWorkspace,
    handleSaveFile, 
    autoSaveEnabled, 
    activeTab, 
    theme
  ]);

  return (
    <div className="app-container">
      {/* TitleBar */}
      <TitleBar
        documentTitle={activeTab?.title || t('app.title')}
        isModified={activeTab?.isModified || false}
        sidebarOpen={sidebarOpen}
        onToggleSidebar={() => setSidebarOpen(prev => !prev)}
        viewMode={viewMode}
        onViewModeChange={setViewMode}
        theme={theme}
        onToggleTheme={() => setTheme(prev => prev === 'dark' ? 'light' : 'dark')}
        onNewFile={handleNewFile}
        onOpenFile={handleOpenFile}
        onSaveFile={() => handleSaveFile(false)}
        onOpenExport={() => setExportModalOpen(true)}
        onOpenCommandPalette={() => setCommandPaletteOpen(true)}
        onOpenSettings={() => setSettingsModalOpen(true)}
      />

      {/* Main Workspace */}
      <div className="main-workspace">
        {/* Sidebar */}
        <div style={{ width: sidebarOpen ? sidebarWidth : 0, display: 'flex', flexShrink: 0 }}>
          <Sidebar
            isOpen={sidebarOpen}
            rootPath={rootFolderPath}
            fileTree={fileTree}
            activeFilePath={activeTab?.filePath}
            outlineItems={outlineItems}
            filterMdOnly={filterMdOnly}
            onOpenFile={handleFileTreeItemClick}
            onOpenFolder={handleOpenFolder}
            onCloseWorkspace={handleCloseWorkspace}
            onNewTab={handleNewFile}
            onCreateFile={handleCreateFileInFolder}
            onCreateFolder={handleCreateFolderInFolder}
            onRenameFile={handleRenameFile}
            onDeleteFile={handleDeleteFileFromTree}
            onShowInFinder={handleShowInFinder}
            onRefreshTree={handleRefreshTree}
            onSelectHeading={handleSelectHeading}
          />
        </div>

        {/* Sidebar Resizer Handle */}
        {sidebarOpen && (
          <div 
            className={`sidebar-resizer ${isDraggingSidebar ? 'dragging' : ''}`}
            onMouseDown={handleSidebarMouseDown}
            onDoubleClick={() => setSidebarWidth(260)}
            title="Sidebar Width"
          />
        )}

        {/* Content Area */}
        <div className="content-area">
          {/* Tab Bar */}
          <TabBar
            tabs={tabs}
            activeTabId={activeTabId}
            onSelectTab={(id) => {
              setActiveTabId(id);
              setTimeout(() => editorRef.current?.focus(), 60);
            }}
            onCloseTab={handleCloseTab}
            onNewTab={handleNewFile}
          />

          {/* Default Save Location Setup Prompt Banner (Task 3) */}
          {!defaultSavePath && !dismissedPrompt && (
            <div style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: '8px 16px',
              background: 'linear-gradient(90deg, rgba(37, 99, 235, 0.12) 0%, rgba(56, 189, 248, 0.08) 100%)',
              borderBottom: '1px solid rgba(56, 189, 248, 0.2)',
              fontSize: 12.5,
              color: 'var(--text-primary)',
              animation: 'fadeIn 0.2s ease-out'
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <FolderCheck size={16} color="var(--accent-primary)" />
                <span>{t('app.defaultSaveLocationPrompt')}</span>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <button
                  className="btn-primary"
                  onClick={handleSelectDefaultSavePath}
                  style={{ fontSize: 11.5, padding: '3px 10px' }}
                >
                  {t('app.chooseFolder')}
                </button>
                <button
                  className="btn-icon"
                  onClick={() => setDismissedPrompt(true)}
                  style={{ width: 20, height: 20 }}
                  title={t('app.later')}
                >
                  <X size={12} />
                </button>
              </div>
            </div>
          )}

          {/* Formatting Bar */}
          {viewMode !== 'preview' && (
            <FormattingBar onInsert={handleInsertFormat} />
          )}

          {/* Editor & Preview Split Area */}
          <div 
            ref={splitPaneRef}
            className="editor-preview-split"
            style={{ userSelect: isDraggingSplit || isDraggingSidebar ? 'none' : 'auto' }}
          >
            {/* Editor Pane */}
            {(viewMode === 'editor' || viewMode === 'split') && (
              <div 
                className="editor-pane"
                style={{ 
                  width: viewMode === 'split' ? `calc(${splitRatio}% - 4px)` : '100%', 
                  flex: 'none' 
                }}
              >
                <Editor
                  ref={editorRef}
                  docId={activeTab?.id || 'default-doc'}
                  value={activeTab ? activeTab.content : ''}
                  theme={theme}
                  fontSize={fontSize}
                  baseDir={baseDir}
                  onImageSaved={handleRefreshTree}
                  onChange={handleContentChange}
                  onScroll={handleEditorScroll}
                  onCursorChange={(line, col) => setCursorPos({ line, col })}
                />
              </div>
            )}

            {/* Split Resizer Divider Handle */}
            {viewMode === 'split' && (
              <div 
                className={`split-resizer ${isDraggingSplit ? 'dragging' : ''}`}
                onMouseDown={handleSplitMouseDown}
                onDoubleClick={() => setSplitRatio(50)}
              >
                <div className="split-resizer-line" />
              </div>
            )}

            {/* Preview Pane */}
            {(viewMode === 'preview' || viewMode === 'split') && (
              <div 
                className="preview-pane"
                style={{ 
                  width: viewMode === 'split' ? `calc(${100 - splitRatio}% - 4px)` : '100%', 
                  flex: 'none' 
                }}
              >
                <Preview
                  scrollRef={previewScrollRef}
                  content={activeTab ? activeTab.content : ''}
                  theme={theme}
                  fontSize={fontSize}
                  activeFilePath={activeTab?.filePath}
                  rootPath={rootFolderPath}
                  onToggleTask={handleToggleTask}
                />
              </div>
            )}
          </div>

          {/* Status Bar */}
          <StatusBar 
            stats={stats} 
            cursorPos={cursorPos}
            fontSize={fontSize}
            autoSave={autoSaveEnabled}
            onToggleAutoSave={() => setAutoSaveEnabled(prev => !prev)}
            onZoomIn={handleZoomIn}
            onZoomOut={handleZoomOut}
            onResetZoom={handleResetZoom}
          />
        </div>
      </div>

      {/* Command Palette */}
      <CommandPalette
        isOpen={commandPaletteOpen}
        onClose={() => setCommandPaletteOpen(false)}
        commands={commands}
      />

      {/* Export Modal */}
      <ExportModal
        isOpen={exportModalOpen}
        onClose={() => setExportModalOpen(false)}
        documentTitle={activeTab?.title || 'Document'}
        markdownContent={activeTab ? activeTab.content : ''}
      />

      {/* Settings Modal (Task 4) */}
      <SettingsModal
        isOpen={settingsModalOpen}
        onClose={() => setSettingsModalOpen(false)}
        theme={theme}
        onThemeChange={setTheme}
        fontSize={fontSize}
        onFontSizeChange={setFontSize}
        defaultSavePath={defaultSavePath}
        onSelectDefaultSavePath={handleSelectDefaultSavePath}
        onSetDocumentsDefault={handleSetDocumentsDefault}
        onClearDefaultSavePath={handleClearDefaultSavePath}
        autoSaveEnabled={autoSaveEnabled}
        onToggleAutoSave={() => setAutoSaveEnabled(prev => !prev)}
        autoSaveToDefault={autoSaveToDefault}
        onToggleAutoSaveToDefault={() => setAutoSaveToDefault(prev => !prev)}
        filterMdOnly={filterMdOnly}
        onToggleFilterMdOnly={() => setFilterMdOnly(prev => !prev)}
      />

      {/* Unsaved Workspace / Window Changes Modal (Task 2) */}
      <UnsavedChangesModal
        isOpen={unsavedModalOpen}
        unsavedTabs={unsavedTabs}
        onSaveAllAndClose={handleSaveAllAndClose}
        onDiscardAndClose={handleDiscardAndClose}
        onCancel={handleCancelUnsavedModal}
      />
    </div>
  );
};
