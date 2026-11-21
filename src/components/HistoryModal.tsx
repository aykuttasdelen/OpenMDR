import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  AlertTriangle,
  Clock,
  Copy,
  Eye,
  FileText,
  FolderOpen,
  GitCompare,
  History,
  RotateCcw,
  Trash2,
  X,
} from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import type {
  DeletedFileSummary,
  DiffLine,
  DiffResult,
  HistoryTrigger,
  RecoverFileResult,
  SnapshotSummary,
} from '../types/electron';
import {
  buildCopyTabTitle,
  formatByteSize,
  formatRelativeTimestamp,
} from '../utils/historyUtils';
import { getBasename } from '../utils/markdownUtils';

export interface HistoryModalProps {
  isOpen: boolean;
  initialTab?: 'current' | 'deleted';
  activeFilePath?: string;
  activeFileTitle?: string;
  activeContent: string;
  onClose: () => void;
  onRestored: (filePath: string, restoredContent: string) => void;
  onOpenCopy: (title: string, content: string) => void;
  onRecovered: (result: RecoverFileResult) => void;
}

type HistoryTab = 'current' | 'deleted';
type ViewerMode = 'diff' | 'content';
type BusyAction = 'restoring' | 'copying' | 'recovering' | 'deleting' | null;

const TRIGGER_LABEL_KEYS: Record<HistoryTrigger, string> = {
  initial: 'history.triggerInitial',
  manual: 'history.triggerManual',
  autosave: 'history.triggerAutosave',
  pre_restore: 'history.triggerPreRestore',
};

const DIFF_LINE_CLASS: Record<DiffLine['tag'], string> = {
  insert: 'is-insert',
  delete: 'is-delete',
  equal: 'is-equal',
};

const DIFF_LINE_MARKER: Record<DiffLine['tag'], string> = {
  insert: '+',
  delete: '-',
  equal: ' ',
};

function formatErrorMessage(err: unknown): string {
  if (err instanceof Error && err.message) {
    return err.message;
  }
  if (typeof err === 'string' && err.trim().length > 0) {
    return err;
  }
  return String(err);
}

function formatExactDate(epochMs: number, lang: 'tr' | 'en'): string {
  if (!Number.isFinite(epochMs)) {
    return '';
  }
  return new Date(epochMs).toLocaleString(lang === 'tr' ? 'tr-TR' : 'en-US');
}

export const HistoryModal: React.FC<HistoryModalProps> = ({
  isOpen,
  initialTab = 'current',
  activeFilePath,
  activeFileTitle,
  activeContent,
  onClose,
  onRestored,
  onOpenCopy,
  onRecovered,
}) => {
  const { language, t } = useTranslation();

  const [activeTab, setActiveTab] = useState<HistoryTab>(initialTab);
  const [viewMode, setViewMode] = useState<ViewerMode>('diff');
  const [loadingInitial, setLoadingInitial] = useState(false);

  const [snapshots, setSnapshots] = useState<SnapshotSummary[]>([]);
  const [selectedSnapshotId, setSelectedSnapshotId] = useState<number | null>(null);
  const [diffResult, setDiffResult] = useState<DiffResult | null>(null);
  const [snapshotContent, setSnapshotContent] = useState<string | null>(null);
  const [loadingViewer, setLoadingViewer] = useState(false);

  const [deletedFiles, setDeletedFiles] = useState<DeletedFileSummary[]>([]);
  const [selectedDeletedFileId, setSelectedDeletedFileId] = useState<number | null>(null);
  const [deletedSnapshots, setDeletedSnapshots] = useState<SnapshotSummary[]>([]);
  const [selectedDeletedSnapshotId, setSelectedDeletedSnapshotId] = useState<number | null>(null);
  const [deletedContent, setDeletedContent] = useState<string | null>(null);
  const [loadingDeletedSnapshots, setLoadingDeletedSnapshots] = useState(false);
  const [loadingDeletedContent, setLoadingDeletedContent] = useState(false);

  const [busyAction, setBusyAction] = useState<BusyAction>(null);
  const [error, setError] = useState<string | null>(null);

  const isMountedRef = useRef(true);
  const viewerSeqRef = useRef(0);
  const deletedFileSeqRef = useRef(0);
  const deletedContentSeqRef = useRef(0);

  const api = typeof window !== 'undefined' ? window.electronAPI : undefined;
  const isBusy = busyAction !== null;

  useEffect(() => {
    isMountedRef.current = true;
    return () => {
      isMountedRef.current = false;
    };
  }, []);

  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  useEffect(() => {
    if (!isOpen) {
      viewerSeqRef.current += 1;
      deletedFileSeqRef.current += 1;
      deletedContentSeqRef.current += 1;
      return;
    }

    setActiveTab(initialTab);
    setViewMode('diff');
    setError(null);
    setBusyAction(null);
    setSnapshots([]);
    setSelectedSnapshotId(null);
    setDiffResult(null);
    setSnapshotContent(null);
    setDeletedFiles([]);
    setSelectedDeletedFileId(null);
    setDeletedSnapshots([]);
    setSelectedDeletedSnapshotId(null);
    setDeletedContent(null);

    if (!api) {
      setLoadingInitial(false);
      return;
    }

    let cancelled = false;
    setLoadingInitial(true);

    const snapshotsPromise = activeFilePath
      ? api.historyListSnapshots(activeFilePath)
      : Promise.resolve<SnapshotSummary[]>([]);
    const deletedPromise = api.historyListDeletedFiles();

    Promise.allSettled([snapshotsPromise, deletedPromise]).then(([snapRes, delRes]) => {
      if (cancelled || !isMountedRef.current) return;

      let firstError: string | null = null;

      if (snapRes.status === 'fulfilled') {
        setSnapshots(snapRes.value);
        setSelectedSnapshotId(snapRes.value[0]?.id ?? null);
      } else {
        firstError = formatErrorMessage(snapRes.reason);
      }

      if (delRes.status === 'fulfilled') {
        setDeletedFiles(delRes.value);
        setSelectedDeletedFileId(delRes.value[0]?.fileId ?? null);
      } else if (!firstError) {
        firstError = formatErrorMessage(delRes.reason);
      }

      if (firstError) {
        setError(t('history.loadError', { error: firstError }));
      }
      setLoadingInitial(false);
    });

    return () => {
      cancelled = true;
    };
  }, [isOpen, activeFilePath, initialTab, api, t]);

  useEffect(() => {
    if (!isOpen || activeTab !== 'current' || selectedSnapshotId === null || !api) {
      return;
    }

    let cancelled = false;
    const seq = ++viewerSeqRef.current;

    setLoadingViewer(true);
    if (viewMode === 'diff') {
      setDiffResult(null);
    } else {
      setSnapshotContent(null);
    }

    const loadViewer = async () => {
      try {
        if (viewMode === 'diff') {
          const diff = await api.historyDiffSnapshot(selectedSnapshotId, activeContent);
          if (cancelled || !isMountedRef.current || seq !== viewerSeqRef.current) return;
          setDiffResult(diff);
        } else {
          const content = await api.historyGetSnapshotContent(selectedSnapshotId);
          if (cancelled || !isMountedRef.current || seq !== viewerSeqRef.current) return;
          setSnapshotContent(content);
        }
      } catch (err) {
        if (cancelled || !isMountedRef.current || seq !== viewerSeqRef.current) return;
        setError(t('history.loadError', { error: formatErrorMessage(err) }));
      } finally {
        if (!cancelled && isMountedRef.current && seq === viewerSeqRef.current) {
          setLoadingViewer(false);
        }
      }
    };

    void loadViewer();

    return () => {
      cancelled = true;
    };
  }, [isOpen, activeTab, selectedSnapshotId, viewMode, activeContent, api, t]);

  useEffect(() => {
    if (!isOpen || activeTab !== 'deleted' || selectedDeletedFileId === null || !api) {
      return;
    }

    let cancelled = false;
    const seq = ++deletedFileSeqRef.current;

    setLoadingDeletedSnapshots(true);
    setDeletedSnapshots([]);
    setSelectedDeletedSnapshotId(null);
    setDeletedContent(null);

    const loadFileSnapshots = async () => {
      try {
        const list = await api.historyListSnapshotsByFileId(selectedDeletedFileId);
        if (cancelled || !isMountedRef.current || seq !== deletedFileSeqRef.current) return;
        setDeletedSnapshots(list);
        setSelectedDeletedSnapshotId(list[0]?.id ?? null);
      } catch (err) {
        if (cancelled || !isMountedRef.current || seq !== deletedFileSeqRef.current) return;
        setError(t('history.loadError', { error: formatErrorMessage(err) }));
      } finally {
        if (!cancelled && isMountedRef.current && seq === deletedFileSeqRef.current) {
          setLoadingDeletedSnapshots(false);
        }
      }
    };

    void loadFileSnapshots();

    return () => {
      cancelled = true;
    };
  }, [isOpen, activeTab, selectedDeletedFileId, api, t]);

  useEffect(() => {
    if (!isOpen || activeTab !== 'deleted' || selectedDeletedSnapshotId === null || !api) {
      return;
    }

    let cancelled = false;
    const seq = ++deletedContentSeqRef.current;

    setLoadingDeletedContent(true);
    setDeletedContent(null);

    const loadContent = async () => {
      try {
        const content = await api.historyGetSnapshotContent(selectedDeletedSnapshotId);
        if (cancelled || !isMountedRef.current || seq !== deletedContentSeqRef.current) return;
        setDeletedContent(content);
      } catch (err) {
        if (cancelled || !isMountedRef.current || seq !== deletedContentSeqRef.current) return;
        setError(t('history.loadError', { error: formatErrorMessage(err) }));
      } finally {
        if (!cancelled && isMountedRef.current && seq === deletedContentSeqRef.current) {
          setLoadingDeletedContent(false);
        }
      }
    };

    void loadContent();

    return () => {
      cancelled = true;
    };
  }, [isOpen, activeTab, selectedDeletedSnapshotId, api, t]);

  const selectedSnapshot = useMemo(
    () => snapshots.find(item => item.id === selectedSnapshotId) ?? null,
    [snapshots, selectedSnapshotId]
  );

  const selectedDeletedFile = useMemo(
    () => deletedFiles.find(item => item.fileId === selectedDeletedFileId) ?? null,
    [deletedFiles, selectedDeletedFileId]
  );

  const selectedDeletedSnapshot = useMemo(
    () => deletedSnapshots.find(item => item.id === selectedDeletedSnapshotId) ?? null,
    [deletedSnapshots, selectedDeletedSnapshotId]
  );

  if (!isOpen) return null;

  const displayFileName =
    activeFileTitle?.trim() || (activeFilePath ? getBasename(activeFilePath) : '');

  const handleSelectSnapshot = (snapshotId: number) => {
    if (snapshotId === selectedSnapshotId) return;
    setError(null);
    setDiffResult(null);
    setSnapshotContent(null);
    setSelectedSnapshotId(snapshotId);
  };

  const handleSelectDeletedFile = (fileId: number) => {
    if (fileId === selectedDeletedFileId) return;
    setError(null);
    setDeletedSnapshots([]);
    setSelectedDeletedSnapshotId(null);
    setDeletedContent(null);
    setSelectedDeletedFileId(fileId);
  };

  const handleOpenCopy = async () => {
    if (!api || !selectedSnapshot || isBusy) return;
    setBusyAction('copying');
    setError(null);
    try {
      const content = await api.historyGetSnapshotContent(selectedSnapshot.id);
      const sourceName = activeFilePath
        ? getBasename(activeFilePath)
        : activeFileTitle?.trim() || '';
      const copyTitle = buildCopyTabTitle(sourceName, selectedSnapshot.createdAt, language);
      onOpenCopy(copyTitle, content);
      onClose();
    } catch (err) {
      if (isMountedRef.current) {
        setError(t('history.loadError', { error: formatErrorMessage(err) }));
      }
    } finally {
      if (isMountedRef.current) {
        setBusyAction(null);
      }
    }
  };

  const handleRestore = async () => {
    if (!api || !activeFilePath || !selectedSnapshot || isBusy) return;
    if (!window.confirm(t('history.restoreConfirm'))) return;

    setBusyAction('restoring');
    setError(null);
    try {
      const restored = await api.historyRestoreSnapshot(
        activeFilePath,
        selectedSnapshot.id,
        activeContent
      );
      onRestored(activeFilePath, restored);
      onClose();
    } catch (err) {
      if (isMountedRef.current) {
        setError(t('history.restoreError', { error: formatErrorMessage(err) }));
      }
    } finally {
      if (isMountedRef.current) {
        setBusyAction(null);
      }
    }
  };

  const handleClearCurrentFileHistory = async () => {
    if (!api || snapshots.length === 0 || isBusy) return;
    const targetFileId = selectedSnapshot?.fileId ?? snapshots[0].fileId;
    if (!window.confirm(t('history.clearFileHistoryConfirm'))) return;

    setBusyAction('deleting');
    setError(null);
    try {
      await api.historyDeleteFileHistory(targetFileId);
      if (!isMountedRef.current) return;
      viewerSeqRef.current += 1;
      setSnapshots([]);
      setSelectedSnapshotId(null);
      setDiffResult(null);
      setSnapshotContent(null);
      setDeletedFiles(prev => prev.filter(item => item.fileId !== targetFileId));
    } catch (err) {
      if (isMountedRef.current) {
        setError(t('history.deleteError', { error: formatErrorMessage(err) }));
      }
    } finally {
      if (isMountedRef.current) {
        setBusyAction(null);
      }
    }
  };

  const handleRemoveDeletedFileHistory = async () => {
    if (!api || !selectedDeletedFile || isBusy) return;
    const target = selectedDeletedFile;
    if (!window.confirm(t('history.removeDeletedConfirm', { name: target.fileName }))) return;

    setBusyAction('deleting');
    setError(null);
    try {
      await api.historyDeleteFileHistory(target.fileId);
      if (!isMountedRef.current) return;
      deletedFileSeqRef.current += 1;
      deletedContentSeqRef.current += 1;
      const remaining = deletedFiles.filter(item => item.fileId !== target.fileId);
      setDeletedFiles(remaining);
      setDeletedSnapshots([]);
      setSelectedDeletedSnapshotId(null);
      setDeletedContent(null);
      setSelectedDeletedFileId(remaining[0]?.fileId ?? null);
    } catch (err) {
      if (isMountedRef.current) {
        setError(t('history.deleteError', { error: formatErrorMessage(err) }));
      }
    } finally {
      if (isMountedRef.current) {
        setBusyAction(null);
      }
    }
  };

  const handleRecoverFile = async (chooseFolder: boolean) => {
    if (!api || !selectedDeletedFile || selectedDeletedSnapshotId === null || isBusy) return;
    setBusyAction('recovering');
    setError(null);
    try {
      let targetDir: string | null = null;
      if (chooseFolder) {
        const picked = await api.openDirectoryDialog(false);
        if (!picked?.dirPath) {
          return;
        }
        targetDir = picked.dirPath;
      }

      const recovered = await api.historyRecoverFile(
        selectedDeletedFile.fileId,
        selectedDeletedSnapshotId,
        targetDir
      );
      onRecovered(recovered);
      onClose();
    } catch (err) {
      if (isMountedRef.current) {
        setError(t('history.recoverError', { error: formatErrorMessage(err) }));
      }
    } finally {
      if (isMountedRef.current) {
        setBusyAction(null);
      }
    }
  };

  const renderCurrentFileTab = () => {
    if (!activeFilePath) {
      return (
        <div className="history-empty-state">
          <FileText size={32} />
          <div className="history-empty-title">{t('history.unsavedFileTitle')}</div>
          <div className="history-empty-desc">{t('history.unsavedFileDesc')}</div>
        </div>
      );
    }

    if (loadingInitial) {
      return (
        <div className="history-empty-state">
          <Clock size={28} />
          <div className="history-empty-desc">{t('history.loading')}</div>
        </div>
      );
    }

    if (snapshots.length === 0) {
      return (
        <div className="history-empty-state">
          <History size={32} />
          <div className="history-empty-title">{t('history.noSnapshots')}</div>
          <div className="history-empty-desc">{t('history.noSnapshotsDesc')}</div>
        </div>
      );
    }

    const diffHasChanges =
      diffResult !== null && (diffResult.addedCount > 0 || diffResult.removedCount > 0);

    return (
      <div className="history-body">
        <div className="history-sidebar">
          <div className="history-sidebar-header">
            <span>{t('history.snapshotsCount', { count: snapshots.length })}</span>
            <button
              type="button"
              className="btn-icon history-clear-btn"
              onClick={handleClearCurrentFileHistory}
              disabled={isBusy}
              title={t('history.clearFileHistory')}
              aria-label={t('history.clearFileHistory')}
            >
              <Trash2 size={14} />
            </button>
          </div>

          <div className="history-list" role="listbox" aria-label={t('history.tabCurrentFile')}>
            {snapshots.map(snap => {
              const isSelected = snap.id === selectedSnapshotId;
              return (
                <button
                  type="button"
                  key={snap.id}
                  role="option"
                  aria-selected={isSelected}
                  className={`history-item ${isSelected ? 'selected' : ''}`}
                  onClick={() => handleSelectSnapshot(snap.id)}
                  title={formatExactDate(snap.createdAt, language)}
                >
                  <div className="history-item-top">
                    <span className="history-item-time">
                      <span className="history-item-dot" />
                      <span>{formatRelativeTimestamp(snap.createdAt, language)}</span>
                    </span>
                    <span className={`history-trigger-badge history-trigger-badge--${snap.trigger}`}>
                      {t(TRIGGER_LABEL_KEYS[snap.trigger])}
                    </span>
                  </div>
                  <div className="history-item-meta">
                    <span>{t('history.linesCount', { count: snap.lineCount })}</span>
                    <span>{formatByteSize(snap.sizeBytes)}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div className="history-viewer-pane">
          <div className="history-viewer-toolbar">
            <div className="view-mode-selector">
              <button
                type="button"
                className={`view-mode-btn ${viewMode === 'diff' ? 'active' : ''}`}
                onClick={() => {
                  setError(null);
                  setViewMode('diff');
                }}
              >
                <GitCompare size={13} />
                <span>{t('history.viewDiff')}</span>
              </button>
              <button
                type="button"
                className={`view-mode-btn ${viewMode === 'content' ? 'active' : ''}`}
                onClick={() => {
                  setError(null);
                  setViewMode('content');
                }}
              >
                <Eye size={13} />
                <span>{t('history.viewContent')}</span>
              </button>
            </div>

            {selectedSnapshot && (
              <div className="history-diff-meta">
                {viewMode === 'diff' ? (
                  <>
                    <span>{t('history.diffDirection')}</span>
                    {diffResult && (
                      <span
                        className={`history-diff-stats ${
                          diffHasChanges ? '' : 'history-diff-stats--unchanged'
                        }`}
                      >
                        {diffHasChanges
                          ? t('history.diffStats', {
                              added: diffResult.addedCount,
                              removed: diffResult.removedCount,
                            })
                          : t('history.diffUnchanged')}
                      </span>
                    )}
                  </>
                ) : (
                  <span>
                    {t('history.linesCount', { count: selectedSnapshot.lineCount })} ·{' '}
                    {t('history.wordsCount', { count: selectedSnapshot.wordCount })} ·{' '}
                    {formatByteSize(selectedSnapshot.sizeBytes)}
                  </span>
                )}
              </div>
            )}
          </div>

          {viewMode === 'diff' && diffResult?.truncated && (
            <div className="history-banner history-banner--warning">
              <AlertTriangle size={15} />
              <span>{t('history.diffTruncated')}</span>
            </div>
          )}

          {loadingViewer || (viewMode === 'diff' ? diffResult === null && !error : snapshotContent === null && !error) ? (
            <div className="history-empty-state">
              <div className="history-empty-desc">{t('history.loading')}</div>
            </div>
          ) : viewMode === 'diff' ? (
            diffResult === null ? null : diffResult.lines.length > 0 ? (
              <div className="history-diff-viewer">
                <div className="history-diff-lines">
                  {diffResult.lines.map((line, index) => (
                    <div
                      key={`${index}-${line.oldLineNo ?? 'x'}-${line.newLineNo ?? 'x'}`}
                      className={`history-diff-line ${DIFF_LINE_CLASS[line.tag]}`}
                    >
                      <span className="history-diff-ln">{line.oldLineNo ?? ''}</span>
                      <span className="history-diff-ln">{line.newLineNo ?? ''}</span>
                      <span className="history-diff-marker">{DIFF_LINE_MARKER[line.tag]}</span>
                      <span className="history-diff-code">
                        {line.content.length > 0 ? line.content : ' '}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            ) : (
              <div className="history-empty-state">
                <div className="history-empty-desc">{t('history.diffUnchanged')}</div>
              </div>
            )
          ) : snapshotContent !== null ? (
            <pre className="history-content-viewer">{snapshotContent}</pre>
          ) : null}

          <div className="history-footer">
            <div className="history-footer-actions">
              <button
                type="button"
                className="btn-secondary"
                onClick={handleOpenCopy}
                disabled={!selectedSnapshot || isBusy}
              >
                <Copy size={14} />
                <span>{t('history.openCopyBtn')}</span>
              </button>
              <button
                type="button"
                className="btn-primary"
                onClick={handleRestore}
                disabled={!selectedSnapshot || isBusy}
              >
                <RotateCcw size={14} />
                <span>{t('history.restoreBtn')}</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  };

  const renderDeletedFilesTab = () => {
    if (loadingInitial) {
      return (
        <div className="history-empty-state">
          <Clock size={28} />
          <div className="history-empty-desc">{t('history.loading')}</div>
        </div>
      );
    }

    if (deletedFiles.length === 0) {
      return (
        <div className="history-empty-state">
          <Trash2 size={32} />
          <div className="history-empty-title">{t('history.noDeletedFiles')}</div>
          <div className="history-empty-desc">{t('history.noDeletedFilesDesc')}</div>
        </div>
      );
    }

    return (
      <div className="history-body">
        <div className="history-sidebar">
          <div className="history-sidebar-header">
            <span>{t('history.tabDeletedFiles')}</span>
            <span>{deletedFiles.length}</span>
          </div>

          <div className="history-list" role="listbox" aria-label={t('history.tabDeletedFiles')}>
            {deletedFiles.map(file => {
              const isSelected = file.fileId === selectedDeletedFileId;
              return (
                <button
                  type="button"
                  key={file.fileId}
                  role="option"
                  aria-selected={isSelected}
                  className={`history-item ${isSelected ? 'selected' : ''}`}
                  onClick={() => handleSelectDeletedFile(file.fileId)}
                  title={file.filePath}
                >
                  <div className="history-item-top">
                    <span className="history-item-time">
                      <span className="history-item-dot" />
                      <span>{file.fileName}</span>
                    </span>
                  </div>
                  <div className="history-item-path">{file.filePath}</div>
                  <div className="history-item-meta">
                    <span>{formatRelativeTimestamp(file.updatedAt, language)}</span>
                    <span>{t('history.snapshotsCount', { count: file.snapshotCount })}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>

        <div className="history-viewer-pane">
          {selectedDeletedFile && (
            <div className="history-viewer-toolbar">
              <div className="history-deleted-file-info">
                <div className="history-deleted-file-name">{selectedDeletedFile.fileName}</div>
                <div className="history-deleted-file-path" title={selectedDeletedFile.filePath}>
                  {selectedDeletedFile.filePath}
                </div>
              </div>

              <div className="history-snapshot-picker">
                <select
                  className="history-snapshot-select"
                  aria-label={t('history.selectSnapshot')}
                  value={selectedDeletedSnapshotId ?? ''}
                  onChange={e => {
                    if (!e.target.value) return;
                    const nextId = Number(e.target.value);
                    if (Number.isFinite(nextId)) {
                      setError(null);
                      setSelectedDeletedSnapshotId(nextId);
                    }
                  }}
                  disabled={loadingDeletedSnapshots || deletedSnapshots.length === 0 || isBusy}
                >
                  {deletedSnapshots.map(snap => (
                    <option key={snap.id} value={snap.id}>
                      {formatRelativeTimestamp(snap.createdAt, language)} —{' '}
                      {t(TRIGGER_LABEL_KEYS[snap.trigger])} ({formatByteSize(snap.sizeBytes)})
                    </option>
                  ))}
                </select>

                {selectedDeletedSnapshot && (
                  <span
                    className={`history-trigger-badge history-trigger-badge--${selectedDeletedSnapshot.trigger}`}
                  >
                    {t('history.linesCount', { count: selectedDeletedSnapshot.lineCount })}
                  </span>
                )}
              </div>
            </div>
          )}

          {loadingDeletedSnapshots ||
          loadingDeletedContent ||
          (selectedDeletedSnapshotId !== null && deletedContent === null && !error) ? (
            <div className="history-empty-state">
              <div className="history-empty-desc">{t('history.loading')}</div>
            </div>
          ) : deletedContent !== null ? (
            <pre className="history-content-viewer">{deletedContent}</pre>
          ) : !error ? (
            <div className="history-empty-state">
              <div className="history-empty-desc">{t('history.noSnapshots')}</div>
            </div>
          ) : null}

          <div className="history-footer">
            <button
              type="button"
              className="btn-secondary btn-danger-ghost"
              onClick={handleRemoveDeletedFileHistory}
              disabled={!selectedDeletedFile || isBusy}
            >
              <Trash2 size={14} />
              <span>{t('history.removeDeletedHistory')}</span>
            </button>

            <div className="history-footer-actions">
              <button
                type="button"
                className="btn-secondary"
                onClick={() => handleRecoverFile(true)}
                disabled={!selectedDeletedFile || selectedDeletedSnapshotId === null || isBusy}
              >
                <FolderOpen size={14} />
                <span>{t('history.recoverToFolder')}</span>
              </button>
              <button
                type="button"
                className="btn-primary"
                onClick={() => handleRecoverFile(false)}
                disabled={!selectedDeletedFile || selectedDeletedSnapshotId === null || isBusy}
              >
                <RotateCcw size={14} />
                <span>{t('history.recoverOriginal')}</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div
        className="palette-box history-modal-box"
        role="dialog"
        aria-modal="true"
        aria-labelledby="history-modal-title"
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div className="history-header">
          <div className="history-header-title">
            <History size={18} color="var(--accent-primary)" />
            <h3 id="history-modal-title">{t('history.title')}</h3>
            {activeTab === 'current' && displayFileName && (
              <span className="history-file-pill" title={activeFilePath || displayFileName}>
                {displayFileName}
              </span>
            )}
          </div>
          <button
            type="button"
            className="btn-icon"
            onClick={onClose}
            style={{ width: 26, height: 26 }}
            title={t('history.close')}
            aria-label={t('history.close')}
          >
            <X size={16} />
          </button>
        </div>

        {!api ? (
          <>
            <div className="history-empty-state">
              <History size={34} />
              <div className="history-empty-desc">{t('history.electronOnly')}</div>
            </div>
            <div className="history-footer">
              <div className="history-footer-actions">
                <button type="button" className="btn-secondary" onClick={onClose}>
                  {t('history.close')}
                </button>
              </div>
            </div>
          </>
        ) : (
          <>
            {/* Navigation Tabs */}
            <div className="history-tabs" role="tablist">
              <button
                type="button"
                role="tab"
                aria-selected={activeTab === 'current'}
                className={`settings-tab-btn ${activeTab === 'current' ? 'active' : ''}`}
                onClick={() => {
                  setError(null);
                  setActiveTab('current');
                }}
              >
                <FileText size={14} />
                <span>{t('history.tabCurrentFile')}</span>
                {activeFilePath && !loadingInitial && (
                  <span className="history-tab-count">{snapshots.length}</span>
                )}
              </button>
              <button
                type="button"
                role="tab"
                aria-selected={activeTab === 'deleted'}
                className={`settings-tab-btn ${activeTab === 'deleted' ? 'active' : ''}`}
                onClick={() => {
                  setError(null);
                  setActiveTab('deleted');
                }}
              >
                <Trash2 size={14} />
                <span>{t('history.tabDeletedFiles')}</span>
                {!loadingInitial && (
                  <span className="history-tab-count">{deletedFiles.length}</span>
                )}
              </button>
            </div>

            {error && (
              <div className="history-banner history-banner--error" role="alert">
                <AlertTriangle size={15} />
                <span>{error}</span>
              </div>
            )}

            {activeTab === 'current' ? renderCurrentFileTab() : renderDeletedFilesTab()}
          </>
        )}
      </div>
    </div>
  );
};
