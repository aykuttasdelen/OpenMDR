import React, { useState } from 'react';
import { 
  X, 
  Settings as SettingsIcon, 
  Globe, 
  FolderCheck, 
  Type, 
  Info, 
  Moon, 
  Sun, 
  Check, 
  FolderOpen, 
  Trash2, 
  FileText,
  Sliders
} from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import { ThemeMode } from '../types/electron';

interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  theme: ThemeMode;
  onThemeChange: (theme: ThemeMode) => void;
  fontSize: number;
  onFontSizeChange: (size: number) => void;
  defaultSavePath: string | null;
  onSelectDefaultSavePath: () => Promise<void>;
  onClearDefaultSavePath: () => void;
  autoSaveEnabled: boolean;
  onToggleAutoSave: () => void;
  autoSaveToDefault: boolean;
  onToggleAutoSaveToDefault: () => void;
  filterMdOnly: boolean;
  onToggleFilterMdOnly: () => void;
}

type TabType = 'general' | 'files' | 'editor' | 'about';

export const SettingsModal: React.FC<SettingsModalProps> = ({
  isOpen,
  onClose,
  theme,
  onThemeChange,
  fontSize,
  onFontSizeChange,
  defaultSavePath,
  onSelectDefaultSavePath,
  onClearDefaultSavePath,
  autoSaveEnabled,
  onToggleAutoSave,
  autoSaveToDefault,
  onToggleAutoSaveToDefault,
  filterMdOnly,
  onToggleFilterMdOnly
}) => {
  const { language, setLanguage, t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TabType>('general');

  if (!isOpen) return null;

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div 
        className="palette-box settings-modal-box" 
        style={{ width: 620, maxWidth: '92vw' }}
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '16px 20px',
          borderBottom: '1px solid var(--border-subtle)',
          background: 'var(--bg-elevated)'
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <SettingsIcon size={18} color="var(--accent-primary)" />
            <h3 style={{ fontSize: 16, fontWeight: 600 }}>{t('settings.title')}</h3>
          </div>
          <button 
            className="btn-icon" 
            onClick={onClose}
            style={{ width: 26, height: 26 }}
            title={t('settings.close')}
          >
            <X size={16} />
          </button>
        </div>

        {/* Settings Navigation Tabs */}
        <div style={{
          display: 'flex',
          borderBottom: '1px solid var(--border-subtle)',
          padding: '0 16px',
          background: 'var(--bg-sidebar)',
          gap: 6
        }}>
          <button
            className={`settings-tab-btn ${activeTab === 'general' ? 'active' : ''}`}
            onClick={() => setActiveTab('general')}
          >
            <Globe size={14} />
            <span>{t('settings.tabGeneral')}</span>
          </button>
          <button
            className={`settings-tab-btn ${activeTab === 'files' ? 'active' : ''}`}
            onClick={() => setActiveTab('files')}
          >
            <FolderCheck size={14} />
            <span>{t('settings.tabFiles')}</span>
          </button>
          <button
            className={`settings-tab-btn ${activeTab === 'editor' ? 'active' : ''}`}
            onClick={() => setActiveTab('editor')}
          >
            <Type size={14} />
            <span>{t('settings.tabEditor')}</span>
          </button>
          <button
            className={`settings-tab-btn ${activeTab === 'about' ? 'active' : ''}`}
            onClick={() => setActiveTab('about')}
          >
            <Info size={14} />
            <span>{t('settings.tabAbout')}</span>
          </button>
        </div>

        {/* Tab Content */}
        <div style={{ padding: '20px', maxHeight: '58vh', overflowY: 'auto' }}>
          {/* GENERAL TAB */}
          {activeTab === 'general' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
              {/* Language Selection */}
              <div className="settings-row">
                <div className="settings-info">
                  <div className="settings-label">{t('settings.language')}</div>
                  <div className="settings-desc">{t('settings.languageDesc')}</div>
                </div>
                <div style={{ display: 'flex', gap: 8 }}>
                  <button
                    className={`btn-secondary ${language === 'tr' ? 'btn-selected' : ''}`}
                    onClick={() => setLanguage('tr')}
                    style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 14px' }}
                  >
                    <span>🇹🇷 Türkçe</span>
                    {language === 'tr' && <Check size={14} color="#38bdf8" />}
                  </button>
                  <button
                    className={`btn-secondary ${language === 'en' ? 'btn-selected' : ''}`}
                    onClick={() => setLanguage('en')}
                    style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 14px' }}
                  >
                    <span>🇬🇧 English</span>
                    {language === 'en' && <Check size={14} color="#38bdf8" />}
                  </button>
                </div>
              </div>

              {/* Theme Selection */}
              <div className="settings-row">
                <div className="settings-info">
                  <div className="settings-label">{t('settings.theme')}</div>
                  <div className="settings-desc">{t('settings.themeDesc')}</div>
                </div>
                <div style={{ display: 'flex', gap: 8 }}>
                  <button
                    className={`btn-secondary ${theme === 'dark' ? 'btn-selected' : ''}`}
                    onClick={() => onThemeChange('dark')}
                    style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 14px' }}
                  >
                    <Moon size={14} />
                    <span>{t('settings.themeDark')}</span>
                    {theme === 'dark' && <Check size={14} color="#38bdf8" />}
                  </button>
                  <button
                    className={`btn-secondary ${theme === 'light' ? 'btn-selected' : ''}`}
                    onClick={() => onThemeChange('light')}
                    style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 14px' }}
                  >
                    <Sun size={14} />
                    <span>{t('settings.themeLight')}</span>
                    {theme === 'light' && <Check size={14} color="#38bdf8" />}
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* FILES & STORAGE TAB */}
          {activeTab === 'files' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
              {/* Default Save Location */}
              <div style={{
                background: 'var(--bg-input)',
                border: '1px solid var(--border-default)',
                borderRadius: 'var(--radius-md)',
                padding: '14px 16px',
                display: 'flex',
                flexDirection: 'column',
                gap: 10
              }}>
                <div>
                  <div className="settings-label">{t('settings.defaultSavePath')}</div>
                  <div className="settings-desc">{t('settings.defaultSavePathDesc')}</div>
                </div>

                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 10,
                  background: 'var(--bg-surface)',
                  padding: '8px 12px',
                  borderRadius: 'var(--radius-sm)',
                  border: '1px solid var(--border-subtle)',
                  fontSize: 12.5,
                  color: defaultSavePath ? 'var(--text-primary)' : 'var(--text-muted)',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap'
                }}>
                  <FolderOpen size={16} color="var(--accent-primary)" style={{ flexShrink: 0 }} />
                  <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis' }}>
                    {defaultSavePath || t('settings.defaultSavePathNotSet')}
                  </span>
                </div>

                <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
                  {defaultSavePath && (
                    <button
                      className="btn-secondary"
                      onClick={onClearDefaultSavePath}
                      style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 12px', color: 'var(--danger-color)' }}
                    >
                      <Trash2 size={13} />
                      <span>{t('settings.clear')}</span>
                    </button>
                  )}
                  <button
                    className="btn-primary"
                    onClick={onSelectDefaultSavePath}
                    style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 14px' }}
                  >
                    <FolderOpen size={14} />
                    <span>{t('settings.browse')}</span>
                  </button>
                </div>
              </div>

              {/* Auto Save To Default Path Toggle */}
              <div className="settings-row">
                <div className="settings-info">
                  <div className="settings-label">{t('settings.autoSaveToDefault')}</div>
                  <div className="settings-desc">{t('settings.autoSaveToDefaultDesc')}</div>
                </div>
                <label className="toggle-switch">
                  <input
                    type="checkbox"
                    checked={autoSaveToDefault}
                    onChange={onToggleAutoSaveToDefault}
                    disabled={!defaultSavePath}
                  />
                  <span className="toggle-slider" />
                </label>
              </div>

              {/* Auto Save General Toggle */}
              <div className="settings-row">
                <div className="settings-info">
                  <div className="settings-label">{t('settings.autoSave')}</div>
                  <div className="settings-desc">{t('settings.autoSaveDesc')}</div>
                </div>
                <label className="toggle-switch">
                  <input
                    type="checkbox"
                    checked={autoSaveEnabled}
                    onChange={onToggleAutoSave}
                  />
                  <span className="toggle-slider" />
                </label>
              </div>

              {/* Only .md and .mdx Filter Toggle */}
              <div className="settings-row">
                <div className="settings-info">
                  <div className="settings-label">{t('settings.filterMdOnly')}</div>
                  <div className="settings-desc">{t('settings.filterMdOnlyDesc')}</div>
                </div>
                <label className="toggle-switch">
                  <input
                    type="checkbox"
                    checked={filterMdOnly}
                    onChange={onToggleFilterMdOnly}
                  />
                  <span className="toggle-slider" />
                </label>
              </div>
            </div>
          )}

          {/* EDITOR TAB */}
          {activeTab === 'editor' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 20 }}>
              {/* Font Size */}
              <div className="settings-row">
                <div className="settings-info">
                  <div className="settings-label">{t('settings.fontSize')}</div>
                  <div className="settings-desc">{t('settings.fontSizeDesc')}</div>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <input
                    type="range"
                    min="11"
                    max="26"
                    value={fontSize}
                    onChange={e => onFontSizeChange(parseInt(e.target.value, 10))}
                    style={{ width: 120, accentColor: 'var(--accent-primary)' }}
                  />
                  <span style={{ 
                    fontFamily: 'var(--font-editor)', 
                    fontSize: 13, 
                    minWidth: 42,
                    textAlign: 'right' 
                  }}>
                    {fontSize}px
                  </span>
                </div>
              </div>
            </div>
          )}

          {/* ABOUT TAB */}
          {activeTab === 'about' && (
            <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', textAlign: 'center', padding: '16px 0', gap: 12 }}>
              <div style={{
                width: 60,
                height: 60,
                borderRadius: 16,
                background: 'linear-gradient(135deg, #38bdf8 0%, #2563eb 100%)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                boxShadow: '0 8px 24px rgba(37, 99, 235, 0.35)'
              }}>
                <FileText size={32} color="#ffffff" />
              </div>
              <div>
                <h2 style={{ fontSize: 20, fontWeight: 700, margin: '4px 0' }}>OpenMD</h2>
                <div style={{ fontSize: 13, color: 'var(--text-muted)' }}>
                  {t('settings.aboutVersion')} 1.0.0
                </div>
              </div>
              <p style={{ fontSize: 13.5, color: 'var(--text-secondary)', maxWidth: 360, lineHeight: 1.5 }}>
                {t('settings.aboutDesc')}
              </p>
              <div style={{ fontSize: 12, color: 'var(--text-muted)', marginTop: 8 }}>
                Built with Electron, React, TypeScript & CodeMirror 6
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          display: 'flex',
          justifyContent: 'flex-end',
          padding: '12px 20px',
          borderTop: '1px solid var(--border-subtle)',
          background: 'var(--bg-elevated)'
        }}>
          <button 
            className="btn-secondary" 
            onClick={onClose}
            style={{ padding: '6px 18px' }}
          >
            {t('settings.close')}
          </button>
        </div>
      </div>
    </div>
  );
};
