import React, { useEffect } from 'react';
import { AlertTriangle, CloudOff, CloudUpload, RotateCcw, X } from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import { mapDriveErrorCode } from '../utils/driveUtils';

export interface DriveQuitSyncModalProps {
  isOpen: boolean;
  pendingCount: number;
  phase: 'syncing' | 'timeout' | 'error';
  errorMessage?: string | null;
  onSkipAndClose: () => void;
  onRetry: () => void;
  onCancel: () => void;
}

export const DriveQuitSyncModal: React.FC<DriveQuitSyncModalProps> = ({
  isOpen,
  pendingCount,
  phase,
  errorMessage,
  onSkipAndClose,
  onRetry,
  onCancel,
}) => {
  const { t } = useTranslation();

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onCancel();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onCancel]);

  if (!isOpen) return null;

  const isSyncing = phase === 'syncing';
  const titleText =
    phase === 'timeout'
      ? t('driveQuit.timeoutTitle')
      : phase === 'error'
      ? t('driveQuit.errorTitle')
      : t('driveQuit.title');

  const descriptionText =
    phase === 'timeout'
      ? t('driveQuit.timeoutDesc', { count: pendingCount })
      : phase === 'error'
      ? t('driveQuit.offlineDesc', { count: pendingCount })
      : t('driveQuit.syncingDesc', { count: pendingCount });

  const badgeTone =
    phase === 'timeout' ? 'warning' : phase === 'error' ? 'danger' : 'info';

  const formattedError =
    errorMessage && errorMessage.trim().length > 0
      ? mapDriveErrorCode(errorMessage, t)
      : null;

  return (
    <div
      className="modal-overlay"
      onClick={() => {
        if (!isSyncing) {
          onCancel();
        }
      }}
      style={{ zIndex: 1200 }}
    >
      <div
        className="palette-box drive-quit-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="drive-quit-modal-title"
        onClick={e => e.stopPropagation()}
      >
        <div className="drive-quit-header">
          <div className="drive-quit-header-left">
            <div className={`drive-quit-icon-badge drive-quit-icon-badge--${badgeTone}`}>
              {phase === 'timeout' ? (
                <AlertTriangle size={18} />
              ) : phase === 'error' ? (
                <CloudOff size={18} />
              ) : (
                <CloudUpload size={18} />
              )}
            </div>
            <h3 id="drive-quit-modal-title" style={{ fontSize: 15.5, fontWeight: 600 }}>
              {titleText}
            </h3>
          </div>
          <button
            type="button"
            className="btn-icon"
            onClick={onCancel}
            style={{ width: 26, height: 26 }}
            title={t('driveQuit.cancel')}
            aria-label={t('driveQuit.cancel')}
          >
            <X size={16} />
          </button>
        </div>

        <div className="drive-quit-body">
          <p className="drive-quit-desc">{descriptionText}</p>

          {isSyncing && (
            <div className="drive-quit-progress-track" role="progressbar" aria-busy="true">
              <div className="drive-quit-progress-bar" />
            </div>
          )}

          {!isSyncing && formattedError && (
            <div className="drive-quit-error-box" role="alert">
              {formattedError}
            </div>
          )}
        </div>

        <div className="drive-quit-footer">
          <button type="button" className="btn-secondary" onClick={onCancel}>
            {t('driveQuit.cancel')}
          </button>

          <div className="drive-quit-footer-actions">
            {isSyncing ? (
              <button
                type="button"
                className="btn-secondary btn-danger-ghost"
                onClick={onSkipAndClose}
              >
                {t('driveQuit.skipAndClose')}
              </button>
            ) : (
              <>
                <button
                  type="button"
                  className="btn-secondary btn-danger-ghost"
                  onClick={onSkipAndClose}
                >
                  {t('driveQuit.closeAnyway')}
                </button>
                <button type="button" className="btn-primary" onClick={onRetry}>
                  <RotateCcw size={14} />
                  <span>{t('driveQuit.retry')}</span>
                </button>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
};
