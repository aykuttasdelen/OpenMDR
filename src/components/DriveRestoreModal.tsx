import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  CheckCircle2,
  Cloud,
  FileText,
  FolderDown,
  FolderOpen,
  Loader2,
  PlusSquare,
  RefreshCw,
  Search,
  X,
} from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import type {
  DriveBatchRestoreResult,
  DriveRemoteFile,
  DriveRestoreResult,
} from '../types/electron';
import { filterRemoteFiles, mapDriveErrorCode } from '../utils/driveUtils';
import { formatByteSize, formatRelativeTimestamp } from '../utils/historyUtils';

export interface DriveRestoreModalProps {
  isOpen: boolean;
  onClose: () => void;
  onOpenInNewTab: (fileName: string, content: string) => void;
  onRestoredToDisk: (result: DriveRestoreResult) => void;
  onBatchRestored?: (result: DriveBatchRestoreResult) => void;
}

type BusyAction = 'opening' | 'restoring' | 'batchRestoring' | null;

interface RestoreBanner {
  kind: 'success' | 'warning';
  message: string;
}

function getRemoteFolderLabel(remotePath: string): string {
  const lastSlash = remotePath.lastIndexOf('/');
  if (lastSlash > 0) {
    return remotePath.slice(0, lastSlash);
  }
  return 'OpenMD/';
}

export const DriveRestoreModal: React.FC<DriveRestoreModalProps> = ({
  isOpen,
  onClose,
  onOpenInNewTab,
  onRestoredToDisk,
  onBatchRestored,
}) => {
  const { language, t } = useTranslation();

  const [files, setFiles] = useState<DriveRemoteFile[]>([]);
  const [searchQuery, setSearchQuery] = useState('');
  const [selectedFileId, setSelectedFileId] = useState<string | null>(null);
  const [previewContent, setPreviewContent] = useState<string | null>(null);
  const [preserveFolders, setPreserveFolders] = useState(true);
  const [loadingList, setLoadingList] = useState(false);
  const [loadingContent, setLoadingContent] = useState(false);
  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [error, setError] = useState<string | null>(null);
  const [banner, setBanner] = useState<RestoreBanner | null>(null);

  const isMountedRef = useRef(true);
  const isOpenRef = useRef(isOpen);
  isOpenRef.current = isOpen;
  const openSessionRef = useRef(0);
  const listSeqRef = useRef(0);
  const selectionSeqRef = useRef(0);

  const api = typeof window !== 'undefined' ? window.electronAPI : undefined;
  const hasDriveRestoreApi = Boolean(api?.driveListRemoteFiles && api?.driveDownloadFileContent);
  const isBusy = busyAction !== null;

  const isSessionActive = useCallback(
    (session: number): boolean =>
      isMountedRef.current && isOpenRef.current && session === openSessionRef.current,
    []
  );

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      openSessionRef.current += 1;
      isMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        if (!isBusy) {
          onClose();
        }
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, isBusy, onClose]);

  const loadRemoteFiles = useCallback(
    async (session: number) => {
      if (!api?.driveListRemoteFiles) {
        setLoadingList(false);
        return;
      }

      const seq = ++listSeqRef.current;
      setLoadingList(true);
      setError(null);

      try {
        const remoteFiles = await api.driveListRemoteFiles();
        if (!isSessionActive(session) || seq !== listSeqRef.current) return;

        setFiles(remoteFiles);
        setSelectedFileId(prev =>
          prev && remoteFiles.some(item => item.fileId === prev)
            ? prev
            : remoteFiles[0]?.fileId ?? null
        );
      } catch (err) {
        if (!isSessionActive(session) || seq !== listSeqRef.current) return;
        setError(mapDriveErrorCode(err, t));
      } finally {
        if (isSessionActive(session) && seq === listSeqRef.current) {
          setLoadingList(false);
        }
      }
    },
    [api, isSessionActive, t]
  );

  useEffect(() => {
    if (!isOpen) {
      openSessionRef.current += 1;
      listSeqRef.current += 1;
      selectionSeqRef.current += 1;
      setBusyAction(null);
      return;
    }

    const session = ++openSessionRef.current;
    setFiles([]);
    setSearchQuery('');
    setSelectedFileId(null);
    setPreviewContent(null);
    setPreserveFolders(true);
    setError(null);
    setBanner(null);
    setBusyAction(null);

    if (!hasDriveRestoreApi) {
      setLoadingList(false);
      return;
    }

    void loadRemoteFiles(session);
  }, [isOpen, hasDriveRestoreApi, loadRemoteFiles]);

  const filteredFiles = useMemo(
    () => filterRemoteFiles(files, searchQuery),
    [files, searchQuery]
  );

  const effectiveSelectedFile = useMemo(
    () =>
      filteredFiles.find(item => item.fileId === selectedFileId) ??
      filteredFiles[0] ??
      null,
    [filteredFiles, selectedFileId]
  );

  const effectiveSelectedFileId = effectiveSelectedFile?.fileId ?? null;

  useEffect(() => {
    if (
      !isOpen ||
      loadingList ||
      !effectiveSelectedFileId ||
      !api?.driveDownloadFileContent
    ) {
      selectionSeqRef.current += 1;
      setPreviewContent(null);
      setLoadingContent(false);
      return;
    }

    let cancelled = false;
    const session = openSessionRef.current;
    const seq = ++selectionSeqRef.current;

    setLoadingContent(true);
    setPreviewContent(null);

    const downloadPreview = async () => {
      try {
        const content = await api.driveDownloadFileContent!(effectiveSelectedFileId);
        if (
          cancelled ||
          !isSessionActive(session) ||
          seq !== selectionSeqRef.current
        ) {
          return;
        }
        setPreviewContent(content);
      } catch (err) {
        if (
          cancelled ||
          !isSessionActive(session) ||
          seq !== selectionSeqRef.current
        ) {
          return;
        }
        setError(mapDriveErrorCode(err, t));
      } finally {
        if (
          !cancelled &&
          isSessionActive(session) &&
          seq === selectionSeqRef.current
        ) {
          setLoadingContent(false);
        }
      }
    };

    void downloadPreview();

    return () => {
      cancelled = true;
    };
  }, [isOpen, loadingList, effectiveSelectedFileId, api, isSessionActive, t]);

  if (!isOpen) return null;

  const handleRequestClose = () => {
    if (isBusy) return;
    onClose();
  };

  const handleRefresh = () => {
    if (isBusy || loadingList) return;
    setBanner(null);
    void loadRemoteFiles(openSessionRef.current);
  };

  const handleSelectFile = (fileId: string) => {
    if (isBusy || fileId === effectiveSelectedFileId) return;
    setError(null);
    setPreviewContent(null);
    setSelectedFileId(fileId);
  };

  const handleOpenInNewTab = async () => {
    if (!effectiveSelectedFile || isBusy) return;
    const session = openSessionRef.current;
    const targetFile = effectiveSelectedFile;

    if (previewContent !== null) {
      onOpenInNewTab(targetFile.name, previewContent);
      onClose();
      return;
    }

    if (!api?.driveDownloadFileContent) return;

    setBusyAction('opening');
    setError(null);
    try {
      const content = await api.driveDownloadFileContent(targetFile.fileId);
      if (!isSessionActive(session)) return;
      onOpenInNewTab(targetFile.name, content);
      onClose();
    } catch (err) {
      if (isSessionActive(session)) {
        setError(mapDriveErrorCode(err, t));
      }
    } finally {
      if (isSessionActive(session)) {
        setBusyAction(null);
      }
    }
  };

  const handleRestoreToFolder = async () => {
    if (
      !api?.openDirectoryDialog ||
      !api?.driveRestoreFile ||
      !effectiveSelectedFile ||
      isBusy
    ) {
      return;
    }

    const session = openSessionRef.current;
    const targetFile = effectiveSelectedFile;
    const keepFolders = preserveFolders;

    setBusyAction('restoring');
    setError(null);
    setBanner(null);

    try {
      const picked = await api.openDirectoryDialog(false);
      if (!isSessionActive(session) || !picked?.dirPath) {
        return;
      }

      const result = await api.driveRestoreFile(
        targetFile.fileId,
        targetFile.remotePath,
        picked.dirPath,
        keepFolders
      );
      if (!isSessionActive(session)) return;

      onRestoredToDisk(result);
      onClose();
    } catch (err) {
      if (isSessionActive(session)) {
        setError(mapDriveErrorCode(err, t));
      }
    } finally {
      if (isSessionActive(session)) {
        setBusyAction(null);
      }
    }
  };

  const handleRestoreAll = async () => {
    if (
      !api?.openDirectoryDialog ||
      !api?.driveRestoreAll ||
      files.length === 0 ||
      isBusy
    ) {
      return;
    }

    const session = openSessionRef.current;
    const keepFolders = preserveFolders;

    setBusyAction('batchRestoring');
    setError(null);
    setBanner(null);

    try {
      const picked = await api.openDirectoryDialog(false);
      if (!isSessionActive(session) || !picked?.dirPath) {
        return;
      }

      const result = await api.driveRestoreAll(picked.dirPath, keepFolders);
      if (!isSessionActive(session)) return;

      onBatchRestored?.(result);

      if (result.failedFiles.length > 0) {
        setBanner({
          kind: 'warning',
          message: t('driveRestore.batchPartialError', {
            restored: result.restoredCount,
            failed: result.failedFiles.length,
            skipped: result.skippedCount,
          }),
        });
      } else {
        setBanner({
          kind: 'success',
          message: t('driveRestore.batchSuccess', {
            restored: result.restoredCount,
            skipped: result.skippedCount,
          }),
        });
      }
    } catch (err) {
      if (isSessionActive(session)) {
        setError(mapDriveErrorCode(err, t));
      }
    } finally {
      if (isSessionActive(session)) {
        setBusyAction(null);
      }
    }
  };

  return (
    <div className="modal-overlay" onClick={handleRequestClose}>
      <div
        className="palette-box drive-restore-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="drive-restore-modal-title"
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div className="history-header">
          <div className="history-header-title">
            <Cloud size={18} color="var(--accent-primary)" />
            <h3 id="drive-restore-modal-title">{t('driveRestore.title')}</h3>
            {hasDriveRestoreApi && !loadingList && (
              <span className="history-file-pill">
                {t('driveRestore.fileCount', { count: filteredFiles.length })}
              </span>
            )}
          </div>
          <button
            type="button"
            className="btn-icon"
            onClick={handleRequestClose}
            disabled={isBusy}
            style={{ width: 26, height: 26 }}
            title={t('driveRestore.close')}
            aria-label={t('driveRestore.close')}
          >
            <X size={16} />
          </button>
        </div>

        {!hasDriveRestoreApi ? (
          <>
            <div className="history-empty-state">
              <Cloud size={34} />
              <div className="history-empty-desc">{t('history.electronOnly')}</div>
            </div>
            <div className="drive-restore-footer">
              <div className="history-footer-actions">
                <button type="button" className="btn-secondary" onClick={handleRequestClose}>
                  {t('driveRestore.close')}
                </button>
              </div>
            </div>
          </>
        ) : (
          <>
            {/* Top Toolbar */}
            <div className="drive-restore-toolbar">
              <div className="drive-restore-search">
                <Search size={14} />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={e => setSearchQuery(e.target.value)}
                  placeholder={t('driveRestore.searchPlaceholder')}
                  aria-label={t('driveRestore.searchPlaceholder')}
                  disabled={isBusy}
                />
                {searchQuery && (
                  <button
                    type="button"
                    className="btn-icon"
                    onClick={() => setSearchQuery('')}
                    disabled={isBusy}
                    style={{ width: 20, height: 20 }}
                    aria-label={t('settings.clear')}
                  >
                    <X size={12} />
                  </button>
                )}
              </div>

              <button
                type="button"
                className="btn-secondary"
                onClick={handleRefresh}
                disabled={isBusy || loadingList}
              >
                <RefreshCw size={14} className={loadingList ? 'drive-spin' : undefined} />
                <span>{t('driveRestore.refresh')}</span>
              </button>

              <button
                type="button"
                className="btn-primary"
                onClick={handleRestoreAll}
                disabled={isBusy || loadingList || files.length === 0 || !api?.driveRestoreAll}
              >
                {busyAction === 'batchRestoring' ? (
                  <Loader2 size={14} className="drive-spin" />
                ) : (
                  <FolderDown size={14} />
                )}
                <span>{t('driveRestore.downloadAll')}</span>
              </button>
            </div>

            {error && (
              <div className="history-banner history-banner--error" role="alert">
                <AlertTriangle size={15} />
                <span>{error}</span>
              </div>
            )}

            {banner && (
              <div
                className={`history-banner ${
                  banner.kind === 'warning'
                    ? 'history-banner--warning'
                    : 'cloud-banner--success'
                }`}
                role="status"
              >
                {banner.kind === 'warning' ? (
                  <AlertTriangle size={15} />
                ) : (
                  <CheckCircle2 size={15} />
                )}
                <span>{banner.message}</span>
              </div>
            )}

            {loadingList ? (
              <div className="history-empty-state">
                <Loader2 size={28} className="drive-spin" />
                <div className="history-empty-desc">{t('driveRestore.loading')}</div>
              </div>
            ) : files.length === 0 ? (
              !error ? (
                <div className="history-empty-state">
                  <Cloud size={34} />
                  <div className="history-empty-title">{t('driveRestore.empty')}</div>
                </div>
              ) : (
                <div style={{ flex: 1 }} />
              )
            ) : (
              <div className="drive-restore-body">
                {/* Left Sidebar: File List */}
                <div className="drive-restore-sidebar">
                  <div className="history-sidebar-header">
                    <span>OpenMD/</span>
                    <span>{t('driveRestore.fileCount', { count: filteredFiles.length })}</span>
                  </div>

                  {filteredFiles.length === 0 ? (
                    <div className="history-empty-state" style={{ padding: '24px 16px' }}>
                      <Search size={24} />
                      <div className="history-empty-desc">{t('command.noResults')}</div>
                    </div>
                  ) : (
                    <div
                      className="drive-restore-list"
                      role="listbox"
                      aria-label={t('driveRestore.title')}
                    >
                      {filteredFiles.map(file => {
                        const isSelected = file.fileId === effectiveSelectedFileId;
                        const folderLabel = getRemoteFolderLabel(file.remotePath);
                        const timeLabel =
                          file.modifiedTimeMs != null
                            ? formatRelativeTimestamp(file.modifiedTimeMs, language)
                            : '';

                        return (
                          <button
                            type="button"
                            key={file.fileId}
                            role="option"
                            aria-selected={isSelected}
                            disabled={isBusy}
                            className={`drive-restore-item ${isSelected ? 'selected' : ''}`}
                            onClick={() => handleSelectFile(file.fileId)}
                            title={file.remotePath}
                          >
                            <div className="drive-restore-item-name">
                              <FileText size={13} style={{ flexShrink: 0 }} />
                              <span>{file.name}</span>
                            </div>
                            <div className="drive-restore-item-path">{folderLabel}</div>
                            <div className="drive-restore-item-meta">
                              <span>{formatByteSize(file.sizeBytes)}</span>
                              {timeLabel && <span>{timeLabel}</span>}
                            </div>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>

                {/* Right Pane: Preview */}
                <div className="drive-restore-preview-pane">
                  {effectiveSelectedFile ? (
                    <>
                      <div className="drive-restore-preview-header">
                        <div className="history-deleted-file-info">
                          <div className="history-deleted-file-name">
                            {effectiveSelectedFile.name}
                          </div>
                          <div
                            className="history-deleted-file-path"
                            title={effectiveSelectedFile.remotePath}
                          >
                            {effectiveSelectedFile.remotePath}
                          </div>
                        </div>
                        <span className="history-file-pill">
                          {t('driveRestore.preview')} ·{' '}
                          {formatByteSize(effectiveSelectedFile.sizeBytes)}
                        </span>
                      </div>

                      {loadingContent || (previewContent === null && !error) ? (
                        <div className="history-empty-state">
                          <Loader2 size={24} className="drive-spin" />
                          <div className="history-empty-desc">
                            {t('driveRestore.loadingContent')}
                          </div>
                        </div>
                      ) : previewContent !== null ? (
                        <pre className="drive-restore-preview-content">{previewContent}</pre>
                      ) : null}
                    </>
                  ) : (
                    <div className="history-empty-state">
                      <FileText size={30} />
                      <div className="history-empty-desc">{t('driveRestore.selectPrompt')}</div>
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* Footer Actions */}
            <div className="drive-restore-footer">
              <label className="drive-restore-checkbox">
                <input
                  type="checkbox"
                  checked={preserveFolders}
                  onChange={e => setPreserveFolders(e.target.checked)}
                  disabled={isBusy}
                />
                <span>{t('driveRestore.preserveFolders')}</span>
              </label>

              <div className="history-footer-actions">
                <button
                  type="button"
                  className="btn-secondary"
                  onClick={handleOpenInNewTab}
                  disabled={!effectiveSelectedFile || isBusy || loadingList}
                >
                  {busyAction === 'opening' ? (
                    <Loader2 size={14} className="drive-spin" />
                  ) : (
                    <PlusSquare size={14} />
                  )}
                  <span>{t('driveRestore.openInNewTab')}</span>
                </button>
                <button
                  type="button"
                  className="btn-primary"
                  onClick={handleRestoreToFolder}
                  disabled={
                    !effectiveSelectedFile ||
                    isBusy ||
                    loadingList ||
                    !api?.openDirectoryDialog ||
                    !api?.driveRestoreFile
                  }
                >
                  {busyAction === 'restoring' ? (
                    <Loader2 size={14} className="drive-spin" />
                  ) : (
                    <FolderOpen size={14} />
                  )}
                  <span>{t('driveRestore.restoreToFolder')}</span>
                </button>
              </div>
            </div>
          </>
        )}
      </div>
    </div>
  );
};
