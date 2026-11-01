import React from 'react';
import { AlignLeft, Clock, FileCode, Type, CheckCircle2, CircleDashed, ZoomIn, ZoomOut } from 'lucide-react';
import { DocumentStats } from '../utils/markdownUtils';
import { useTranslation } from '../i18n/LanguageContext';

interface StatusBarProps {
  stats: DocumentStats;
  cursorPos: { line: number; col: number };
  fontSize?: number;
  autoSave?: boolean;
  onToggleAutoSave?: () => void;
  onZoomIn?: () => void;
  onZoomOut?: () => void;
  onResetZoom?: () => void;
}

export const StatusBar: React.FC<StatusBarProps> = React.memo(({ 
  stats, 
  cursorPos,
  fontSize = 14,
  autoSave = false,
  onToggleAutoSave,
  onZoomIn,
  onZoomOut,
  onResetZoom
}) => {
  const { t } = useTranslation();

  return (
    <footer className="status-bar">
      <div className="status-bar-left">
        <div className="status-bar-item">
          <span>{t('status.cursor', { line: cursorPos.line, col: cursorPos.col })}</span>
        </div>
        <div className="status-bar-item">
          <Type size={12} />
          <span>{t('status.words', { count: stats.words.toLocaleString() })}</span>
        </div>
        <div className="status-bar-item">
          <AlignLeft size={12} />
          <span>{t('status.chars', { count: stats.chars.toLocaleString() })}</span>
        </div>
        <div className="status-bar-item">
          <Clock size={12} />
          <span>{t('status.readingTime', { time: stats.readingTime })}</span>
        </div>
      </div>

      <div className="status-bar-right">
        {onToggleAutoSave && (
          <button 
            className="status-bar-btn"
            onClick={onToggleAutoSave}
            title={t('status.autoSaveToggle', { 
              status: autoSave ? t('status.autoSaveOn') : t('status.autoSaveOff') 
            })}
          >
            {autoSave ? (
              <CheckCircle2 size={12} color="#10b981" />
            ) : (
              <CircleDashed size={12} color="var(--text-muted)" />
            )}
            <span style={{ color: autoSave ? '#10b981' : 'inherit' }}>
              {t('status.autoSave')}: {autoSave ? t('status.autoSaveOn') : t('status.autoSaveOff')}
            </span>
          </button>
        )}

        <div className="status-bar-item" style={{ gap: 4 }}>
          {onZoomOut && (
            <button className="status-bar-zoom-btn" onClick={onZoomOut} title={t('status.zoomOut')}>
              <ZoomOut size={11} />
            </button>
          )}
          <span 
            onClick={onResetZoom} 
            title={t('status.zoomReset')} 
            style={{ cursor: onResetZoom ? 'pointer' : 'default', padding: '0 2px' }}
          >
            {fontSize}px
          </span>
          {onZoomIn && (
            <button className="status-bar-zoom-btn" onClick={onZoomIn} title={t('status.zoomIn')}>
              <ZoomIn size={11} />
            </button>
          )}
        </div>

        <div className="status-bar-item">
          <span>UTF-8</span>
        </div>
        <div className="status-bar-item">
          <FileCode size={12} />
          <span>Markdown (GFM)</span>
        </div>
      </div>
    </footer>
  );
});
