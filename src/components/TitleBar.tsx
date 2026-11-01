import React from 'react';
import { 
  Sidebar as SidebarIcon, 
  Columns2, 
  FileEdit, 
  Eye, 
  Sun, 
  Moon, 
  Save, 
  FolderOpen, 
  FilePlus, 
  Share2, 
  Command,
  Settings as SettingsIcon
} from 'lucide-react';
import { ViewMode, ThemeMode } from '../types/electron';
import { useTranslation } from '../i18n/LanguageContext';

interface TitleBarProps {
  documentTitle: string;
  isModified: boolean;
  sidebarOpen: boolean;
  onToggleSidebar: () => void;
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  theme: ThemeMode;
  onToggleTheme: () => void;
  onNewFile: () => void;
  onOpenFile: () => void;
  onSaveFile: () => void;
  onOpenExport: () => void;
  onOpenCommandPalette: () => void;
  onOpenSettings: () => void;
}

export const TitleBar: React.FC<TitleBarProps> = React.memo(({
  documentTitle,
  isModified,
  sidebarOpen,
  onToggleSidebar,
  viewMode,
  onViewModeChange,
  theme,
  onToggleTheme,
  onNewFile,
  onOpenFile,
  onSaveFile,
  onOpenExport,
  onOpenCommandPalette,
  onOpenSettings
}) => {
  const { t } = useTranslation();

  const handleDoubleClick = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement;
    if (target.closest('button') || target.closest('input') || target.closest('select')) {
      return;
    }
    if (window.electronAPI?.toggleMaximize) {
      window.electronAPI.toggleMaximize();
    }
  };

  return (
    <header className="titlebar" data-tauri-drag-region onDoubleClick={handleDoubleClick}>
      {/* Left section: traffic lights spacer + left toolbar buttons */}
      <div className="titlebar-left-wrapper">
        <div className="titlebar-traffic-lights-spacer" />
        <div className="titlebar-left">
          <button 
            className={`btn-icon ${sidebarOpen ? 'active' : ''}`}
            onClick={onToggleSidebar}
            title={t('titlebar.sidebarToggle')}
          >
            <SidebarIcon size={16} />
          </button>
          <button 
            className="btn-icon" 
            onClick={onNewFile}
            title={t('titlebar.newDoc')}
          >
            <FilePlus size={16} />
          </button>
          <button 
            className="btn-icon" 
            onClick={onOpenFile}
            title={t('titlebar.openDoc')}
          >
            <FolderOpen size={16} />
          </button>
          <button 
            className="btn-icon" 
            onClick={onSaveFile}
            title={t('titlebar.saveDoc')}
          >
            <Save size={16} />
          </button>
        </div>
      </div>

      {/* Center: App Title */}
      <div className="titlebar-center" data-tauri-drag-region>
        <div className="doc-title-badge app-title-badge" data-tauri-drag-region>
          <span className="app-title-text" data-tauri-drag-region>OpenMD</span>
        </div>
      </div>

      {/* Right controls */}
      <div className="titlebar-right">
        {/* View mode selector */}
        <div className="view-mode-selector">
          <button 
            className={`view-mode-btn ${viewMode === 'editor' ? 'active' : ''}`}
            onClick={() => onViewModeChange('editor')}
            title={t('titlebar.editorOnly')}
          >
            <FileEdit size={14} />
            <span>{t('titlebar.editor')}</span>
          </button>
          <button 
            className={`view-mode-btn ${viewMode === 'split' ? 'active' : ''}`}
            onClick={() => onViewModeChange('split')}
            title={t('titlebar.splitView')}
          >
            <Columns2 size={14} />
            <span>{t('titlebar.split')}</span>
          </button>
          <button 
            className={`view-mode-btn ${viewMode === 'preview' ? 'active' : ''}`}
            onClick={() => onViewModeChange('preview')}
            title={t('titlebar.previewOnly')}
          >
            <Eye size={14} />
            <span>{t('titlebar.preview')}</span>
          </button>
        </div>

        {/* Command palette */}
        <button 
          className="btn-icon"
          onClick={onOpenCommandPalette}
          title={t('titlebar.commandPalette')}
        >
          <Command size={16} />
        </button>

        {/* Export */}
        <button 
          className="btn-icon"
          onClick={onOpenExport}
          title={t('titlebar.export')}
        >
          <Share2 size={16} />
        </button>

        {/* Settings button */}
        <button 
          className="btn-icon"
          onClick={onOpenSettings}
          title={t('titlebar.settings')}
        >
          <SettingsIcon size={16} />
        </button>

        {/* Theme toggle */}
        <button 
          className="btn-icon"
          onClick={onToggleTheme}
          title={theme === 'dark' ? t('titlebar.themeLight') : t('titlebar.themeDark')}
        >
          {theme === 'dark' ? <Sun size={16} /> : <Moon size={16} />}
        </button>
      </div>
    </header>
  );
});
