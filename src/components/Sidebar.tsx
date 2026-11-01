import React, { useState, useRef, useEffect } from 'react';
import { 
  Folder, 
  FolderOpen, 
  FileText, 
  ChevronRight, 
  ChevronDown, 
  FolderPlus, 
  FilePlus, 
  Trash2, 
  RefreshCw, 
  ListTree, 
  Files, 
  Search, 
  Hash, 
  Pencil, 
  ExternalLink, 
  Check, 
  X,
  XCircle,
  FileCheck2
} from 'lucide-react';
import { FileItem, OutlineItem } from '../types/electron';
import { getBasename, getDirname, isMarkdownFileName } from '../utils/markdownUtils';
import { useTranslation } from '../i18n/LanguageContext';

interface SidebarProps {
  isOpen: boolean;
  rootPath?: string;
  fileTree: FileItem[];
  activeFilePath?: string;
  outlineItems: OutlineItem[];
  filterMdOnly?: boolean;
  onOpenFile: (path: string, name: string) => void;
  onOpenFolder: () => void;
  onCloseWorkspace?: () => void;
  onNewTab: () => void;
  onCreateFile: (parentDir: string, fileName: string) => Promise<void>;
  onCreateFolder: (parentDir: string, folderName: string) => Promise<void>;
  onRenameFile: (oldPath: string, newPath: string) => Promise<void>;
  onDeleteFile: (path: string) => void;
  onShowInFinder?: (path: string) => void;
  onRefreshTree: () => void;
  onSelectHeading: (line: number) => void;
}

interface InlineCreationState {
  type: 'file' | 'folder';
  parentPath: string;
}

export const Sidebar: React.FC<SidebarProps> = React.memo(({
  isOpen,
  rootPath,
  fileTree,
  activeFilePath,
  outlineItems,
  filterMdOnly = true,
  onOpenFile,
  onOpenFolder,
  onCloseWorkspace,
  onNewTab,
  onCreateFile,
  onCreateFolder,
  onRenameFile,
  onDeleteFile,
  onShowInFinder,
  onRefreshTree,
  onSelectHeading
}) => {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState<'files' | 'outline'>('files');
  const [searchFilter, setSearchFilter] = useState('');
  const [expandedFolders, setExpandedFolders] = useState<Record<string, boolean>>({});

  // Inline creation & rename state
  const [creationState, setCreationState] = useState<InlineCreationState | null>(null);
  const [creationName, setCreationName] = useState('');
  const [renamingPath, setRenamingPath] = useState<string | null>(null);
  const [renamingName, setRenamingName] = useState('');

  const creationInputRef = useRef<HTMLInputElement>(null);
  const renameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (creationState) {
      creationInputRef.current?.focus();
      creationInputRef.current?.select();
    }
  }, [creationState]);

  useEffect(() => {
    if (renamingPath) {
      renameInputRef.current?.focus();
      renameInputRef.current?.select();
    }
  }, [renamingPath]);

  const toggleFolder = (path: string) => {
    setExpandedFolders(prev => ({
      ...prev,
      [path]: !prev[path]
    }));
  };

  const startCreate = (type: 'file' | 'folder', parentPath?: string) => {
    const targetParent = parentPath || rootPath;
    if (!targetParent) {
      if (type === 'file') {
        onNewTab();
      } else {
        onOpenFolder();
      }
      return;
    }
    setCreationState({ type, parentPath: targetParent });
    setCreationName(type === 'file' ? `${t('app.untitled')}.md` : t('sidebar.defaultFolderName'));
    setExpandedFolders(prev => ({ ...prev, [targetParent]: true }));
  };

  const submitCreate = async () => {
    if (!creationState || !creationName.trim()) {
      setCreationState(null);
      return;
    }
    const { type, parentPath } = creationState;
    const name = creationName.trim();
    setCreationState(null);
    try {
      if (type === 'file') {
        await onCreateFile(parentPath, name);
      } else {
        await onCreateFolder(parentPath, name);
      }
    } catch (err: any) {
      alert(t('app.errorPrefix', { error: err.message }));
    }
  };

  const startRename = (path: string, currentName: string) => {
    setRenamingPath(path);
    setRenamingName(currentName);
  };

  const submitRename = async () => {
    if (!renamingPath || !renamingName.trim()) {
      setRenamingPath(null);
      return;
    }
    const oldPath = renamingPath;
    const dir = getDirname(oldPath);
    const newPath = dir ? `${dir}/${renamingName.trim()}` : renamingName.trim();
    setRenamingPath(null);

    if (oldPath !== newPath) {
      try {
        await onRenameFile(oldPath, newPath);
      } catch (err: any) {
        alert(err.message);
      }
    }
  };

  const renderInlineCreation = (parentPath: string, depth: number) => {
    if (!creationState || creationState.parentPath !== parentPath) return null;

    return (
      <div 
        style={{ 
          paddingLeft: `${depth * 12 + 16}px`, 
          display: 'flex', 
          alignItems: 'center', 
          gap: 6,
          paddingTop: 4,
          paddingBottom: 4
        }}
      >
        {creationState.type === 'file' ? (
          <FileText size={14} color="#38bdf8" />
        ) : (
          <Folder size={14} color="#60a5fa" />
        )}
        <input
          ref={creationInputRef}
          type="text"
          value={creationName}
          onChange={e => setCreationName(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') submitCreate();
            if (e.key === 'Escape') setCreationState(null);
          }}
          onBlur={submitCreate}
          style={{
            flex: 1,
            background: 'var(--bg-input)',
            border: '1px solid var(--accent-primary)',
            borderRadius: 'var(--radius-sm)',
            color: 'var(--text-primary)',
            fontSize: 12,
            padding: '2px 6px',
            outline: 'none'
          }}
        />
        <button 
          className="btn-icon" 
          style={{ width: 18, height: 18 }} 
          onMouseDown={(e) => { e.preventDefault(); submitCreate(); }}
          onClick={submitCreate}
        >
          <Check size={11} color="#10b981" />
        </button>
        <button 
          className="btn-icon" 
          style={{ width: 18, height: 18 }} 
          onMouseDown={(e) => { e.preventDefault(); setCreationState(null); }}
          onClick={() => setCreationState(null)}
        >
          <X size={11} color="#ef4444" />
        </button>
      </div>
    );
  };

  const renderTree = (items: FileItem[], depth = 0) => {
    return items
      .filter(item => {
        if (!item.isDirectory && filterMdOnly) {
          // Task 1: Strict markdown filtering (.md and .mdx)
          if (!isMarkdownFileName(item.name)) {
            return false;
          }
        }
        if (!searchFilter) return true;
        return item.name.toLowerCase().includes(searchFilter.toLowerCase());
      })
      .map(item => {
        const isExpanded = expandedFolders[item.path] !== false;
        const isMarkdown = isMarkdownFileName(item.name);
        const isActive = activeFilePath === item.path;
        const isRenaming = renamingPath === item.path;

        if (item.isDirectory) {
          return (
            <div key={item.path} style={{ paddingLeft: `${depth * 12}px` }}>
              <div 
                className="file-tree-item"
                onClick={() => toggleFolder(item.path)}
              >
                <div className="file-tree-item-name">
                  {isExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  {isExpanded ? <FolderOpen size={15} color="#60a5fa" /> : <Folder size={15} color="#60a5fa" />}
                  {isRenaming ? (
                    <input
                      ref={renameInputRef}
                      type="text"
                      value={renamingName}
                      onClick={e => e.stopPropagation()}
                      onChange={e => setRenamingName(e.target.value)}
                      onKeyDown={e => {
                        if (e.key === 'Enter') submitRename();
                        if (e.key === 'Escape') setRenamingPath(null);
                      }}
                      onBlur={submitRename}
                      style={{
                        background: 'var(--bg-input)',
                        border: '1px solid var(--accent-primary)',
                        borderRadius: 3,
                        color: 'var(--text-primary)',
                        fontSize: 12,
                        padding: '1px 4px',
                        outline: 'none'
                      }}
                    />
                  ) : (
                    <span>{item.name}</span>
                  )}
                </div>

                {/* Folder action buttons */}
                <div style={{ display: 'flex', gap: 2 }} onClick={e => e.stopPropagation()}>
                  <button 
                    className="btn-icon"
                    style={{ width: 18, height: 18, opacity: 0.6 }}
                    title={t('sidebar.newFileInFolder')}
                    onClick={() => startCreate('file', item.path)}
                  >
                    <FilePlus size={12} />
                  </button>
                  <button 
                    className="btn-icon"
                    style={{ width: 18, height: 18, opacity: 0.6 }}
                    title={t('sidebar.newFolderInFolder')}
                    onClick={() => startCreate('folder', item.path)}
                  >
                    <FolderPlus size={12} />
                  </button>
                  <button 
                    className="btn-icon"
                    style={{ width: 18, height: 18, opacity: 0.6 }}
                    title={t('sidebar.rename')}
                    onClick={() => startRename(item.path, item.name)}
                  >
                    <Pencil size={11} />
                  </button>
                  {onShowInFinder && (
                    <button 
                      className="btn-icon"
                      style={{ width: 18, height: 18, opacity: 0.6 }}
                      title={t('sidebar.showInFinder')}
                      onClick={() => onShowInFinder(item.path)}
                    >
                      <ExternalLink size={11} />
                    </button>
                  )}
                  <button 
                    className="btn-icon"
                    style={{ width: 18, height: 18, opacity: 0.6 }}
                    title={t('sidebar.delete')}
                    onClick={() => {
                      if (window.confirm(t('sidebar.confirmDeleteFolder', { name: item.name }))) {
                        onDeleteFile(item.path);
                      }
                    }}
                  >
                    <Trash2 size={11} color="#ef4444" />
                  </button>
                </div>
              </div>

              {/* Subfolder contents and inline create inside folder */}
              {isExpanded && (
                <div>
                  {renderInlineCreation(item.path, depth + 1)}
                  {item.children && renderTree(item.children, depth + 1)}
                </div>
              )}
            </div>
          );
        }

        return (
          <div 
            key={item.path} 
            style={{ paddingLeft: `${depth * 12 + 16}px` }}
            className={`file-tree-item ${isActive ? 'active' : ''}`}
            onClick={() => onOpenFile(item.path, item.name)}
          >
            <div className="file-tree-item-name">
              <FileText size={14} color={isMarkdown ? '#38bdf8' : '#94a3b8'} />
              {isRenaming ? (
                <input
                  ref={renameInputRef}
                  type="text"
                  value={renamingName}
                  onClick={e => e.stopPropagation()}
                  onChange={e => setRenamingName(e.target.value)}
                  onKeyDown={e => {
                    if (e.key === 'Enter') submitRename();
                    if (e.key === 'Escape') setRenamingPath(null);
                  }}
                  onBlur={submitRename}
                  style={{
                    background: 'var(--bg-input)',
                    border: '1px solid var(--accent-primary)',
                    borderRadius: 3,
                    color: 'var(--text-primary)',
                    fontSize: 12,
                    padding: '1px 4px',
                    outline: 'none'
                  }}
                />
              ) : (
                <span>{item.name}</span>
              )}
            </div>

            {/* File action buttons */}
            <div style={{ display: 'flex', gap: 2 }} onClick={e => e.stopPropagation()}>
              <button 
                className="btn-icon"
                style={{ width: 18, height: 18, opacity: 0.6 }}
                title={t('sidebar.rename')}
                onClick={() => startRename(item.path, item.name)}
              >
                <Pencil size={11} />
              </button>
              {onShowInFinder && (
                <button 
                  className="btn-icon"
                  style={{ width: 18, height: 18, opacity: 0.6 }}
                  title={t('sidebar.showInFinder')}
                  onClick={() => onShowInFinder(item.path)}
                >
                  <ExternalLink size={11} />
                </button>
              )}
              <button 
                className="btn-icon"
                style={{ width: 18, height: 18, opacity: 0.6 }}
                title={t('sidebar.delete')}
                onClick={() => {
                  if (window.confirm(t('sidebar.confirmDeleteFile', { name: item.name }))) {
                    onDeleteFile(item.path);
                  }
                }}
              >
                <Trash2 size={11} color="#ef4444" />
              </button>
            </div>
          </div>
        );
      });
  };

  return (
    <aside className={`sidebar ${isOpen ? '' : 'collapsed'}`}>
      {/* Sidebar Tabs */}
      <div className="sidebar-tabs">
        <button 
          className={`sidebar-tab-btn ${activeTab === 'files' ? 'active' : ''}`}
          onClick={() => setActiveTab('files')}
        >
          <Files size={14} />
          <span>{t('sidebar.filesTab')}</span>
        </button>
        <button 
          className={`sidebar-tab-btn ${activeTab === 'outline' ? 'active' : ''}`}
          onClick={() => setActiveTab('outline')}
        >
          <ListTree size={14} />
          <span>{t('sidebar.outlineTab')} ({outlineItems.length})</span>
        </button>
      </div>

      <div className="sidebar-content">
        {activeTab === 'files' ? (
          <div>
            <div className="sidebar-header-actions">
              <span className="sidebar-section-title">
                {rootPath ? getBasename(rootPath) : t('sidebar.filesTab')}
              </span>
              <div style={{ display: 'flex', gap: 4 }}>
                <button 
                  className="btn-icon" 
                  style={{ width: 24, height: 24 }}
                  onClick={onOpenFolder}
                  title={t('sidebar.openFolder')}
                >
                  <FolderOpen size={14} />
                </button>
                <button 
                  className="btn-icon" 
                  style={{ width: 24, height: 24 }}
                  onClick={() => startCreate('file')}
                  title={rootPath ? t('sidebar.newFile') : t('tabbar.newTab')}
                >
                  <FilePlus size={14} />
                </button>
                <button 
                  className="btn-icon" 
                  style={{ width: 24, height: 24 }}
                  onClick={() => startCreate('folder')}
                  title={t('sidebar.newFolder')}
                >
                  <FolderPlus size={14} />
                </button>
                <button 
                  className="btn-icon" 
                  style={{ width: 24, height: 24 }}
                  onClick={onRefreshTree}
                  title={t('sidebar.refresh')}
                >
                  <RefreshCw size={13} />
                </button>
                {rootPath && onCloseWorkspace && (
                  <button 
                    className="btn-icon" 
                    style={{ width: 24, height: 24 }}
                    onClick={onCloseWorkspace}
                    title={t('sidebar.closeWorkspace')}
                  >
                    <XCircle size={14} color="var(--danger-color)" />
                  </button>
                )}
              </div>
            </div>

            {/* Filter Notice Badge when folder is open */}
            {rootPath && filterMdOnly && (
              <div style={{
                display: 'flex',
                alignItems: 'center',
                gap: 5,
                fontSize: 11,
                color: 'var(--text-muted)',
                padding: '4px 8px',
                marginBottom: 8,
                background: 'rgba(56, 189, 248, 0.08)',
                borderRadius: 'var(--radius-sm)',
                border: '1px solid rgba(56, 189, 248, 0.15)'
              }}>
                <FileCheck2 size={12} color="#38bdf8" />
                <span>{t('sidebar.onlyMarkdownNotice')}</span>
              </div>
            )}

            {/* Search Input */}
            <div style={{
              display: 'flex',
              alignItems: 'center',
              gap: 6,
              background: 'var(--bg-input)',
              border: '1px solid var(--border-subtle)',
              borderRadius: 'var(--radius-sm)',
              padding: '4px 8px',
              marginBottom: 10
            }}>
              <Search size={13} color="var(--text-muted)" />
              <input 
                type="text" 
                placeholder={t('sidebar.searchFiles')}
                value={searchFilter}
                onChange={e => setSearchFilter(e.target.value)}
                style={{
                  background: 'transparent',
                  border: 'none',
                  outline: 'none',
                  color: 'var(--text-primary)',
                  fontSize: 12,
                  width: '100%'
                }}
              />
            </div>

            {/* Root-level inline creation */}
            {rootPath && renderInlineCreation(rootPath, 0)}

            {fileTree.length > 0 ? (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                {renderTree(fileTree)}
              </div>
            ) : (
              <div style={{ 
                padding: '24px 12px', 
                textAlign: 'center', 
                color: 'var(--text-muted)',
                fontSize: 12.5 
              }}>
                <p style={{ marginBottom: 10 }}>{t('sidebar.noOpenFolder')}</p>
                <div style={{ display: 'flex', gap: 8, justifyContent: 'center' }}>
                  <button 
                    className="btn-icon" 
                    onClick={onOpenFolder}
                    style={{ 
                      width: 'auto', 
                      padding: '6px 12px', 
                      background: 'var(--bg-elevated)', 
                      border: '1px solid var(--border-subtle)',
                      fontSize: 12
                    }}
                  >
                    {t('sidebar.openFolder')}
                  </button>
                  <button 
                    className="btn-icon" 
                    onClick={onNewTab}
                    style={{ 
                      width: 'auto', 
                      padding: '6px 12px', 
                      background: 'var(--accent-subtle)', 
                      color: 'var(--accent-primary)',
                      border: '1px solid rgba(59, 130, 246, 0.3)',
                      fontSize: 12
                    }}
                  >
                    {t('sidebar.newFile')}
                  </button>
                </div>
              </div>
            )}
          </div>
        ) : (
          <div>
            <div className="sidebar-header-actions">
              <span className="sidebar-section-title">{t('sidebar.outlineTab')}</span>
            </div>
            {outlineItems.length > 0 ? (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
                {outlineItems.map(item => (
                  <div
                    key={item.id}
                    className="file-tree-item"
                    style={{ paddingLeft: `${(item.level - 1) * 14 + 6}px` }}
                    onClick={() => onSelectHeading(item.line)}
                    title={`L${item.line}`}
                  >
                    <div className="file-tree-item-name" style={{ fontSize: item.level === 1 ? 13 : 12 }}>
                      <Hash size={item.level === 1 ? 14 : 12} color="#3b82f6" />
                      <span>{item.text}</span>
                    </div>
                    <span style={{ fontSize: 10, color: 'var(--text-muted)' }}>L{item.line}</span>
                  </div>
                ))}
              </div>
            ) : (
              <div style={{ 
                padding: '24px 12px', 
                textAlign: 'center', 
                color: 'var(--text-muted)',
                fontSize: 12.5 
              }}>
                {t('sidebar.noHeadingsDesc')}
              </div>
            )}
          </div>
        )}
      </div>
    </aside>
  );
});
