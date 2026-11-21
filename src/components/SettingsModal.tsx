import React, { useEffect, useRef, useState } from 'react';
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
  Sliders,
  History
} from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import { HistorySettings, HistoryStats, ThemeMode } from '../types/electron';
import { clampHistorySettings, formatByteSize } from '../utils/historyUtils';

export interface SettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  theme: ThemeMode;
  onThemeChange: (theme: ThemeMode) => void;
  fontSize: number;
  onFontSizeChange: (size: number) => void;
  defaultSavePath: string | null;
  onSelectDefaultSavePath: () => Promise<void>;
  onSetDocumentsDefault?: () => Promise<void>;
  onClearDefaultSavePath: () => void;
  autoSaveEnabled: boolean;
  onToggleAutoSave: () => void;
  autoSaveToDefault: boolean;
  onToggleAutoSaveToDefault: () => void;
  filterMdOnly: boolean;
  onToggleFilterMdOnly: () => void;
  historySettings?: HistorySettings;
  historyStats?: HistoryStats | null;
  onUpdateHistorySettings?: (next: HistorySettings) => Promise<void>;
  onClearAllHistory?: () => Promise<void>;
}

type TabType = 'general' | 'files' | 'editor' | 'about';

const DEFAULT_HISTORY_SETTINGS: HistorySettings = {
  enabled: true,
  maxVersionsPerFile: 50,
  autosaveIntervalMinutes: 3,
};

const MAX_VERSIONS_PRESETS = [20, 50, 100, 200] as const;
const AUTOSAVE_INTERVAL_OPTIONS = [1, 3, 5, 10, 15, 30] as const;

function formatErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) {
    return err.message;
  }
  if (typeof err === 'string' && err.trim().length > 0) {
    return err;
  }
  return String(err);
}

export const SettingsModal: React.FC<SettingsModalProps> = ({
  isOpen,
  onClose,
  theme,
  onThemeChange,
  fontSize,
  onFontSizeChange,
  defaultSavePath,
  onSelectDefaultSavePath,
  onSetDocumentsDefault,
  onClearDefaultSavePath,
  autoSaveEnabled,
  onToggleAutoSave,
  autoSaveToDefault,
  onToggleAutoSaveToDefault,
  filterMdOnly,
  onToggleFilterMdOnly,
  historySettings,
  historyStats,
  onUpdateHistorySettings,
  onClearAllHistory
}) => {
  const { language, setLanguage, t } = useTranslation();
  const [activeTab, setActiveTab] = useState<TabType>('general');

  const currentHistorySettings = historySettings ?? DEFAULT_HISTORY_SETTINGS;
  const [maxVersionsDraft, setMaxVersionsDraft] = useState<string>(
    String(currentHistorySettings.maxVersionsPerFile)
  );
  const [historyBusy, setHistoryBusy] = useState(false);
  const [historyFeedback, setHistoryFeedback] = useState<{
    kind: 'success' | 'error';
    message: string;
  } | null>(null);

  const isMountedRef = useRef(true);
  const historyBusyRef = useRef(false);
  const openSessionRef = useRef(0);

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    openSessionRef.current += 1;
    if (!isOpen) {
      setHistoryFeedback(null);
    }
  }, [isOpen]);

  useEffect(() => {
    setMaxVersionsDraft(String(currentHistorySettings.maxVersionsPerFile));
  }, [currentHistorySettings.maxVersionsPerFile, isOpen]);

  if (!isOpen) return null;

  const canMutateHistory = Boolean(onUpdateHistorySettings) && !historyBusy;
  const canClearHistory = Boolean(onClearAllHistory) && !historyBusy;
  const historyControlsDisabled = !canMutateHistory || !currentHistorySettings.enabled;

  const intervalOptions = AUTOSAVE_INTERVAL_OPTIONS.includes(
    currentHistorySettings.autosaveIntervalMinutes as (typeof AUTOSAVE_INTERVAL_OPTIONS)[number]
  )
    ? [...AUTOSAVE_INTERVAL_OPTIONS]
    : [...AUTOSAVE_INTERVAL_OPTIONS, currentHistorySettings.autosaveIntervalMinutes].sort(
        (a, b) => a - b
      );

  const saveHistorySettings = async (nextSettings: HistorySettings) => {
    if (!onUpdateHistorySettings || historyBusyRef.current) return;
    const clamped = clampHistorySettings(nextSettings);
    const sessionId = openSessionRef.current;

    historyBusyRef.current = true;
    setHistoryBusy(true);
    setHistoryFeedback(null);
    try {
      await onUpdateHistorySettings(clamped);
    } catch (err) {
      if (isMountedRef.current && sessionId === openSessionRef.current) {
        setMaxVersionsDraft(String(currentHistorySettings.maxVersionsPerFile));
        setHistoryFeedback({
          kind: 'error',
          message: formatErrorMessage(err),
        });
      }
    } finally {
      historyBusyRef.current = false;
      if (isMountedRef.current) {
        setHistoryBusy(false);
      }
    }
  };

  const handleToggleHistoryEnabled = () => {
    void saveHistorySettings({
      ...currentHistorySettings,
      enabled: !currentHistorySettings.enabled,
    });
  };

  const commitMaxVersionsDraft = () => {
    if (historyBusyRef.current) return;
    const trimmed = maxVersionsDraft.trim();
    if (!trimmed) {
      setMaxVersionsDraft(String(currentHistorySettings.maxVersionsPerFile));
      return;
    }
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed)) {
      setMaxVersionsDraft(String(currentHistorySettings.maxVersionsPerFile));
      return;
    }
    const clamped = clampHistorySettings({
      ...currentHistorySettings,
      maxVersionsPerFile: parsed,
    });
    setMaxVersionsDraft(String(clamped.maxVersionsPerFile));
    if (clamped.maxVersionsPerFile !== currentHistorySettings.maxVersionsPerFile) {
      void saveHistorySettings(clamped);
    }
  };

  const applyMaxVersionsPreset = (preset: number) => {
    if (!onUpdateHistorySettings || historyBusyRef.current || !currentHistorySettings.enabled) return;
    const clamped = clampHistorySettings({
      ...currentHistorySettings,
      maxVersionsPerFile: preset,
    });
    setMaxVersionsDraft(String(clamped.maxVersionsPerFile));
    if (clamped.maxVersionsPerFile !== currentHistorySettings.maxVersionsPerFile) {
      void saveHistorySettings(clamped);
    }
  };

  const handleAutosaveIntervalChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const parsed = Number(e.target.value);
    if (!Number.isFinite(parsed)) return;
    const clamped = clampHistorySettings({
      ...currentHistorySettings,
      autosaveIntervalMinutes: parsed,
    });
    if (clamped.autosaveIntervalMinutes !== currentHistorySettings.autosaveIntervalMinutes) {
      void saveHistorySettings(clamped);
    }
  };

  const handleClearAllHistory = async () => {
    if (!onClearAllHistory || historyBusyRef.current) return;
    const confirmed = window.confirm(t('settings.historyClearAllConfirm'));
    if (!confirmed) return;

    const sessionId = openSessionRef.current;
    historyBusyRef.current = true;
    setHistoryBusy(true);
    setHistoryFeedback(null);
    try {
      await onClearAllHistory();
      if (isMountedRef.current && sessionId === openSessionRef.current) {
        setHistoryFeedback({
          kind: 'success',
          message: t('settings.historyCleared'),
        });
      }
    } catch (err) {
      if (isMountedRef.current && sessionId === openSessionRef.current) {
        setHistoryFeedback({
          kind: 'error',
          message: formatErrorMessage(err),
        });
      }
    } finally {
      historyBusyRef.current = false;
      if (isMountedRef.current) {
        setHistoryBusy(false);
      }
    }
  };

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

                <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end', flexWrap: 'wrap' }}>
                  {onSetDocumentsDefault && (
                    <button
                      className="btn-secondary"
                      onClick={onSetDocumentsDefault}
                      style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 12px' }}
                      title="Varsayılan olarak Belgeler klasörünü ayarla"
                    >
                      <FolderCheck size={13} color="var(--accent-primary)" />
                      <span>{t('settings.useDocumentsDefault')}</span>
                    </button>
                  )}
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

              {/* Local Version History Card */}
              <div style={{
                background: 'var(--bg-input)',
                border: '1px solid var(--border-default)',
                borderRadius: 'var(--radius-md)',
                padding: '14px 16px',
                display: 'flex',
                flexDirection: 'column',
                gap: 14
              }}>
                {/* Section Header */}
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <History size={16} color="var(--accent-primary)" />
                  <div className="settings-label">{t('settings.historyTitle')}</div>
                </div>

                {/* Enable Version History Toggle */}
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 16 }}>
                  <div className="settings-info">
                    <div className="settings-label">{t('settings.historyEnabled')}</div>
                    <div className="settings-desc">{t('settings.historyEnabledDesc')}</div>
                  </div>
                  <label className="toggle-switch">
                    <input
                      type="checkbox"
                      aria-label={t('settings.historyEnabled')}
                      checked={currentHistorySettings.enabled}
                      onChange={handleToggleHistoryEnabled}
                      disabled={!canMutateHistory}
                    />
                    <span className="toggle-slider" />
                  </label>
                </div>

                {/* Max Versions per File */}
                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 16,
                  flexWrap: 'wrap',
                  paddingTop: 10,
                  borderTop: '1px solid var(--border-subtle)',
                  opacity: currentHistorySettings.enabled ? 1 : 0.6
                }}>
                  <div className="settings-info" style={{ flex: '1 1 220px' }}>
                    <div className="settings-label">{t('settings.historyMaxVersions')}</div>
                    <div className="settings-desc">{t('settings.historyMaxVersionsDesc')}</div>
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
                    <input
                      type="number"
                      min={5}
                      max={200}
                      aria-label={t('settings.historyMaxVersions')}
                      value={maxVersionsDraft}
                      onChange={e => setMaxVersionsDraft(e.target.value)}
                      onBlur={commitMaxVersionsDraft}
                      onKeyDown={e => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          commitMaxVersionsDraft();
                        }
                      }}
                      disabled={historyControlsDisabled}
                      style={{
                        width: 68,
                        padding: '5px 8px',
                        fontSize: 12.5,
                        fontFamily: 'var(--font-editor)',
                        background: 'var(--bg-surface)',
                        color: 'var(--text-primary)',
                        border: '1px solid var(--border-default)',
                        borderRadius: 'var(--radius-sm)',
                        outline: 'none'
                      }}
                    />
                    {MAX_VERSIONS_PRESETS.map(preset => (
                      <button
                        key={preset}
                        type="button"
                        className={`btn-secondary ${currentHistorySettings.maxVersionsPerFile === preset ? 'btn-selected' : ''}`}
                        onMouseDown={e => e.preventDefault()}
                        onClick={() => applyMaxVersionsPreset(preset)}
                        disabled={historyControlsDisabled}
                        style={{
                          padding: '5px 9px',
                          fontSize: 12,
                          background: 'var(--bg-surface)',
                          color: 'var(--text-secondary)',
                          border: '1px solid var(--border-default)',
                          borderRadius: 'var(--radius-sm)',
                          cursor: historyControlsDisabled ? 'not-allowed' : 'pointer'
                        }}
                      >
                        {preset}
                      </button>
                    ))}
                  </div>
                </div>

                {/* Auto-Save Snapshot Interval */}
                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 16,
                  flexWrap: 'wrap',
                  paddingTop: 10,
                  borderTop: '1px solid var(--border-subtle)',
                  opacity: currentHistorySettings.enabled ? 1 : 0.6
                }}>
                  <div className="settings-info" style={{ flex: '1 1 220px' }}>
                    <div className="settings-label">{t('settings.historyAutosaveInterval')}</div>
                    <div className="settings-desc">{t('settings.historyAutosaveIntervalDesc')}</div>
                  </div>
                  <select
                    aria-label={t('settings.historyAutosaveInterval')}
                    value={currentHistorySettings.autosaveIntervalMinutes}
                    onChange={handleAutosaveIntervalChange}
                    disabled={historyControlsDisabled}
                    style={{
                      background: 'var(--bg-surface)',
                      color: 'var(--text-primary)',
                      border: '1px solid var(--border-default)',
                      borderRadius: 'var(--radius-sm)',
                      padding: '6px 10px',
                      fontSize: 12.5,
                      fontFamily: 'var(--font-ui)',
                      outline: 'none',
                      cursor: historyControlsDisabled ? 'not-allowed' : 'pointer'
                    }}
                  >
                    {intervalOptions.map(minutes => (
                      <option key={minutes} value={minutes}>
                        {t('settings.historyMinutes', { count: minutes })}
                      </option>
                    ))}
                  </select>
                </div>

                {/* Storage Stats & Clear All Footer */}
                <div style={{
                  display: 'flex',
                  alignItems: 'center',
                  justifyContent: 'space-between',
                  gap: 12,
                  flexWrap: 'wrap',
                  paddingTop: 10,
                  borderTop: '1px solid var(--border-subtle)'
                }}>
                  <div style={{ fontSize: 12, color: 'var(--text-secondary)' }}>
                    {t('settings.historyStorageStats', {
                      files: historyStats?.totalFiles ?? 0,
                      snapshots: historyStats?.totalSnapshots ?? 0,
                      size: formatByteSize(historyStats?.dbSizeBytes ?? 0),
                    })}
                  </div>
                  <button
                    type="button"
                    className="btn-secondary"
                    onClick={handleClearAllHistory}
                    disabled={!canClearHistory}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 6,
                      padding: '6px 12px',
                      fontSize: 12.5,
                      color: 'var(--danger-color)',
                      background: 'var(--bg-surface)',
                      border: '1px solid rgba(239, 68, 68, 0.35)',
                      borderRadius: 'var(--radius-sm)',
                      cursor: !canClearHistory ? 'not-allowed' : 'pointer',
                      opacity: !canClearHistory ? 0.5 : 1
                    }}
                  >
                    <Trash2 size={13} />
                    <span>{t('settings.historyClearAll')}</span>
                  </button>
                </div>

                {historyFeedback && (
                  <div
                    role={historyFeedback.kind === 'error' ? 'alert' : 'status'}
                    style={{
                      fontSize: 12,
                      padding: '8px 10px',
                      borderRadius: 'var(--radius-sm)',
                      background:
                        historyFeedback.kind === 'error'
                          ? 'var(--danger-subtle)'
                          : 'var(--accent-subtle)',
                      color:
                        historyFeedback.kind === 'error'
                          ? 'var(--danger-color)'
                          : 'var(--accent-primary)',
                      border: `1px solid ${
                        historyFeedback.kind === 'error'
                          ? 'rgba(239, 68, 68, 0.3)'
                          : 'rgba(59, 130, 246, 0.3)'
                      }`
                    }}
                  >
                    {historyFeedback.message}
                  </div>
                )}
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
                  {t('settings.aboutVersion')} 1.1.0
                </div>
              </div>
              <p style={{ fontSize: 13.5, color: 'var(--text-secondary)', maxWidth: 420, lineHeight: 1.5 }}>
                {t('settings.aboutDesc')}
              </p>
              <div style={{ 
                display: 'flex', 
                flexDirection: 'column', 
                gap: 6,
                fontSize: 12, 
                color: 'var(--text-secondary)', 
                marginTop: 6,
                background: 'var(--bg-surface)',
                padding: '12px 18px',
                borderRadius: 'var(--radius-sm)',
                border: '1px solid var(--border-subtle)',
                textAlign: 'left',
                width: '100%',
                maxWidth: 420
              }}>
                <div>⚡ <strong>Mimari:</strong> Rust (Tauri v2) + React 19 + TypeScript + CodeMirror 6</div>
                <div>🍎 <strong>Platform:</strong> macOS (Apple Silicon & Intel) Native WKWebView</div>
                <div>👤 <strong>Geliştirici:</strong> Aykut Taşdelen</div>
                <div>🔗 <strong>Kaynak Kod:</strong> <a href="https://github.com/aykuttasdelen/OpenMD" target="_blank" rel="noreferrer" style={{ color: 'var(--accent-primary)', textDecoration: 'none' }}>github.com/aykuttasdelen/OpenMD</a></div>
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
