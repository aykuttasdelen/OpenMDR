import React from 'react';
import {
  AlertCircle,
  AlignLeft,
  CheckCircle2,
  CircleDashed,
  Clock,
  CloudAlert,
  CloudCheck,
  CloudOff,
  CloudUpload,
  FileCode,
  PauseCircle,
  RefreshCw,
  Type,
  ZoomIn,
  ZoomOut,
} from 'lucide-react';
import { DocumentStats } from '../utils/markdownUtils';
import { useTranslation } from '../i18n/LanguageContext';
import type { DriveStatus, DriveSyncState } from '../types/electron';
import { getDriveStatusTone, mapDriveErrorCode } from '../utils/driveUtils';

interface StatusBarProps {
  stats: DocumentStats;
  cursorPos: { line: number; col: number };
  fontSize?: number;
  autoSave?: boolean;
  onToggleAutoSave?: () => void;
  onZoomIn?: () => void;
  onZoomOut?: () => void;
  onResetZoom?: () => void;
  driveStatus?: DriveStatus | null;
  onOpenDriveSettings?: () => void;
}

function renderDriveIcon(state: DriveSyncState): React.ReactNode {
  switch (state) {
    case 'idle':
      return <CloudCheck size={12} />;
    case 'syncing':
      return <RefreshCw size={12} className="drive-spin" />;
    case 'pending':
      return <CloudUpload size={12} />;
    case 'paused':
      return <PauseCircle size={12} />;
    case 'offline':
      return <CloudOff size={12} />;
    case 'auth_required':
      return <CloudAlert size={12} />;
    case 'error':
      return <AlertCircle size={12} />;
    case 'disconnected':
    default:
      return <CloudOff size={12} />;
  }
}

function getDriveLabel(
  driveStatus: DriveStatus,
  t: (key: string, params?: Record<string, string | number>) => string
): string {
  switch (driveStatus.state) {
    case 'idle':
      return t('status.driveIdle');
    case 'syncing':
      return driveStatus.pendingCount > 0
        ? `${t('status.driveSyncing')} (${driveStatus.pendingCount})`
        : t('status.driveSyncing');
    case 'pending':
      return t('status.drivePending', { count: driveStatus.pendingCount });
    case 'paused':
      return t('status.drivePaused');
    case 'offline':
      return driveStatus.pendingCount > 0
        ? `${t('status.driveOffline')} (${driveStatus.pendingCount})`
        : t('status.driveOffline');
    case 'auth_required':
      return t('status.driveAuthRequired');
    case 'error':
      return t('status.driveError');
    case 'disconnected':
    default:
      return t('status.driveDisconnected');
  }
}

export const StatusBar: React.FC<StatusBarProps> = React.memo(({ 
  stats, 
  cursorPos,
  fontSize = 14,
  autoSave = false,
  onToggleAutoSave,
  onZoomIn,
  onZoomOut,
  onResetZoom,
  driveStatus,
  onOpenDriveSettings,
}) => {
  const { t } = useTranslation();

  const driveTone = driveStatus ? getDriveStatusTone(driveStatus.state) : null;
  const driveLabel = driveStatus ? getDriveLabel(driveStatus, t) : '';
  const driveErrorHint =
    driveStatus &&
    driveStatus.lastError &&
    (driveStatus.state === 'error' ||
      driveStatus.state === 'offline' ||
      driveStatus.state === 'auth_required')
      ? mapDriveErrorCode(driveStatus.lastError, t)
      : '';
  const driveTooltip = driveStatus
    ? driveErrorHint
      ? `${t('status.driveTooltip', { status: driveLabel })} (${driveErrorHint})`
      : t('status.driveTooltip', { status: driveLabel })
    : '';

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
        {driveStatus != null && driveTone != null && (
          <button
            type="button"
            className={`status-bar-cloud-btn status-bar-cloud-btn--${driveTone}`}
            onClick={onOpenDriveSettings}
            title={driveTooltip}
            aria-label={driveTooltip}
            style={{ cursor: onOpenDriveSettings ? 'pointer' : 'default' }}
          >
            {renderDriveIcon(driveStatus.state)}
            <span>{driveLabel}</span>
          </button>
        )}

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
