import React, { useState, useEffect } from 'react';
import { X, FileText, ScanText, Languages, Sparkles, Loader2, AlertCircle, FolderOpen, Images } from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import { OCRProgress } from '../types/electron';
import { getDirname } from '../utils/markdownUtils';

interface OCRModalProps {
  isOpen: boolean;
  onClose: () => void;
  initialPdfPath?: string | null;
  defaultOutputDir?: string | null;
  activeTabFilePath?: string | null;
  onComplete: (markdown: string, originalFileName: string, destination: 'newTab' | 'append', destinationDir?: string) => void;
}

export const OCRModal: React.FC<OCRModalProps> = ({
  isOpen,
  onClose,
  initialPdfPath,
  defaultOutputDir,
  activeTabFilePath,
  onComplete
}) => {
  const { t } = useTranslation();
  const [pdfPath, setPdfPath] = useState<string>('');
  const [pagesMode, setPagesMode] = useState<'all' | 'custom'>('all');
  const [customPages, setCustomPages] = useState<string>('');
  const [language, setLanguage] = useState<string>('tr-TR,en-US');
  const [mode, setMode] = useState<'hybrid' | 'ocr' | 'text'>('hybrid');
  const [destination, setDestination] = useState<'newTab' | 'append'>('newTab');
  const [extractImages, setExtractImages] = useState<boolean>(true);
  const [imagePlacement, setImagePlacement] = useState<'folderOnly' | 'pageEnd' | 'docEnd' | 'inline'>(() => {
    return (localStorage.getItem('openmd_ocr_image_placement') as any) || 'folderOnly';
  });
  const [isProcessing, setIsProcessing] = useState<boolean>(false);
  const [progress, setProgress] = useState<OCRProgress | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      setPdfPath(initialPdfPath || '');
      setErrorMessage(null);
      setProgress(null);
      setIsProcessing(false);
      setExtractImages(true);
    }
  }, [isOpen, initialPdfPath]);

  // Listen to OCR progress events from Tauri
  useEffect(() => {
    if (!isOpen || !window.electronAPI?.onOCRProgress) return;

    const unlisten = window.electronAPI.onOCRProgress((prog) => {
      setProgress(prog);
    });

    return () => {
      unlisten();
    };
  }, [isOpen]);

  if (!isOpen) return null;

  const handlePickFile = async () => {
    if (window.electronAPI?.openPdfDialog) {
      const selected = await window.electronAPI.openPdfDialog();
      if (selected) {
        setPdfPath(selected);
        setErrorMessage(null);
      }
    }
  };

  const getFileName = (pathStr: string) => {
    if (!pathStr) return '';
    const parts = pathStr.split(/[/\\]/);
    return parts[parts.length - 1] || pathStr;
  };

  const handleStartOCR = async () => {
    if (!pdfPath) {
      setErrorMessage(t('ocr.noFileSelected'));
      return;
    }

    if (!window.electronAPI?.processPdfOCR) {
      setErrorMessage('OCR API is not supported in this environment.');
      return;
    }

    try {
      setIsProcessing(true);
      setErrorMessage(null);
      setProgress({ type: 'progress', page: 0, totalPages: 1, percent: 5 });

      let destinationDir = defaultOutputDir || undefined;
      if (destination === 'append' && activeTabFilePath) {
        destinationDir = getDirname(activeTabFilePath);
      } else if (!destinationDir && window.electronAPI) {
        try {
          const docFn = window.electronAPI.getDefaultDocumentsDir || window.electronAPI.getDocumentsDir;
          if (docFn) {
            destinationDir = await docFn();
          }
        } catch (e) {
          console.warn('Failed to resolve default documents dir for OCR assets:', e);
        }
      }
      if (!destinationDir && pdfPath) {
        destinationDir = getDirname(pdfPath);
      }

      const pagesArg = pagesMode === 'all' ? 'all' : customPages.trim() || 'all';

      const result = await window.electronAPI.processPdfOCR({
        pdfPath,
        pages: pagesArg,
        languages: language,
        mode,
        extractImages,
        outputDir: destinationDir,
        imagePlacement,
      });

      if (result.success && (result.markdown || (result.extractedImages && result.extractedImages.length > 0))) {
        onComplete(result.markdown, getFileName(pdfPath), destination, destinationDir);
        onClose();
      } else {
        setErrorMessage(result.error || t('ocr.error', { error: 'Unknown OCR failure' }));
      }
    } catch (err: any) {
      setErrorMessage(t('ocr.error', { error: err.message || String(err) }));
    } finally {
      setIsProcessing(false);
    }
  };

  const fileName = getFileName(pdfPath);

  return (
    <div className="modal-overlay" onClick={() => !isProcessing && onClose()}>
      <div 
        className="palette-box" 
        style={{ width: 520, maxWidth: '92vw' }}
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
            <div style={{
              width: 30,
              height: 30,
              borderRadius: 'var(--radius-sm)',
              background: 'rgba(56, 189, 248, 0.15)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center'
            }}>
              <ScanText size={18} color="#38bdf8" />
            </div>
            <div>
              <h3 style={{ fontSize: 15, fontWeight: 600 }}>{t('ocr.title')}</h3>
              <p style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                macOS Apple Vision & Neural Engine
              </p>
            </div>
          </div>

          <button 
            className="btn-icon" 
            onClick={onClose}
            disabled={isProcessing}
            style={{ width: 26, height: 26 }}
          >
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div style={{ padding: '20px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          {/* Error Banner */}
          {errorMessage && (
            <div style={{
              background: 'var(--danger-subtle)',
              border: '1px solid var(--danger-color)',
              borderRadius: 'var(--radius-sm)',
              padding: '10px 14px',
              fontSize: 13,
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              color: 'var(--danger-color)'
            }}>
              <AlertCircle size={16} style={{ flexShrink: 0 }} />
              <div style={{ flex: 1 }}>{errorMessage}</div>
            </div>
          )}

          {/* File Picker / Selected File Card */}
          <div style={{
            background: 'var(--bg-input)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-md)',
            padding: '12px 14px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            gap: 12
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12, minWidth: 0 }}>
              <div style={{
                width: 36,
                height: 36,
                borderRadius: 'var(--radius-sm)',
                background: 'rgba(239, 68, 68, 0.12)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                flexShrink: 0
              }}>
                <FileText size={20} color="#ef4444" />
              </div>
              <div style={{ minWidth: 0 }}>
                {pdfPath ? (
                  <>
                    <h4 style={{ 
                      fontSize: 13, 
                      fontWeight: 600, 
                      whiteSpace: 'nowrap', 
                      overflow: 'hidden', 
                      textOverflow: 'ellipsis' 
                    }}>
                      {fileName}
                    </h4>
                    <p style={{ 
                      fontSize: 11, 
                      color: 'var(--text-muted)', 
                      whiteSpace: 'nowrap', 
                      overflow: 'hidden', 
                      textOverflow: 'ellipsis' 
                    }}>
                      {pdfPath}
                    </p>
                  </>
                ) : (
                  <>
                    <h4 style={{ fontSize: 13, fontWeight: 500 }}>{t('ocr.selectFile')}</h4>
                    <p style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                      {t('ocr.noFileSelected')}
                    </p>
                  </>
                )}
              </div>
            </div>

            <button
              className="btn-secondary"
              onClick={handlePickFile}
              disabled={isProcessing}
              style={{ fontSize: 12, padding: '5px 12px', flexShrink: 0, display: 'flex', alignItems: 'center', gap: 6 }}
            >
              <FolderOpen size={14} />
              <span>{pdfPath ? t('settings.browse') : t('ocr.choosePdf')}</span>
            </button>
          </div>

          {/* Options Grid */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
            {/* Pages selection */}
            <div>
              <label style={{ display: 'block', fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
                {t('ocr.pages')}
              </label>
              <div style={{ display: 'flex', gap: 8, marginBottom: 6 }}>
                <button
                  type="button"
                  className={pagesMode === 'all' ? 'btn-primary' : 'btn-secondary'}
                  onClick={() => setPagesMode('all')}
                  disabled={isProcessing}
                  style={{ flex: 1, fontSize: 12, padding: '5px 8px' }}
                >
                  {t('ocr.allPages')}
                </button>
                <button
                  type="button"
                  className={pagesMode === 'custom' ? 'btn-primary' : 'btn-secondary'}
                  onClick={() => setPagesMode('custom')}
                  disabled={isProcessing}
                  style={{ flex: 1, fontSize: 12, padding: '5px 8px' }}
                >
                  {t('ocr.customPages')}
                </button>
              </div>

              {pagesMode === 'custom' && (
                <input
                  type="text"
                  className="input-field"
                  placeholder={t('ocr.pagesPlaceholder')}
                  value={customPages}
                  onChange={e => setCustomPages(e.target.value)}
                  disabled={isProcessing}
                  style={{ width: '100%', fontSize: 12, padding: '6px 10px' }}
                />
              )}
            </div>

            {/* Language Selection */}
            <div>
              <label style={{ display: 'block', fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
                {t('ocr.language')}
              </label>
              <select
                className="select-field"
                value={language}
                onChange={e => setLanguage(e.target.value)}
                disabled={isProcessing}
                style={{ width: '100%', fontSize: 12, padding: '6px 10px' }}
              >
                <option value="tr-TR,en-US">{t('ocr.langTrEn')}</option>
                <option value="tr-TR">{t('ocr.langTr')}</option>
                <option value="en-US">{t('ocr.langEn')}</option>
                <option value="de-DE">{t('ocr.langDe')}</option>
                <option value="fr-FR">{t('ocr.langFr')}</option>
              </select>
            </div>
          </div>

          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
            {/* Mode selection */}
            <div>
              <label style={{ display: 'block', fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
                {t('ocr.mode')}
              </label>
              <select
                className="select-field"
                value={mode}
                onChange={e => setMode(e.target.value as any)}
                disabled={isProcessing}
                style={{ width: '100%', fontSize: 12, padding: '6px 10px' }}
              >
                <option value="hybrid">{t('ocr.modeHybrid')}</option>
                <option value="ocr">{t('ocr.modeOcr')}</option>
                <option value="text">{t('ocr.modeText')}</option>
              </select>
            </div>

            {/* Destination Selection */}
            <div>
              <label style={{ display: 'block', fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
                {t('ocr.destination')}
              </label>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  type="button"
                  className={destination === 'newTab' ? 'btn-primary' : 'btn-secondary'}
                  onClick={() => setDestination('newTab')}
                  disabled={isProcessing}
                  style={{ flex: 1, fontSize: 12, padding: '5px 8px' }}
                >
                  {t('ocr.destNewTab')}
                </button>
                <button
                  type="button"
                  className={destination === 'append' ? 'btn-primary' : 'btn-secondary'}
                  onClick={() => setDestination('append')}
                  disabled={isProcessing}
                  style={{ flex: 1, fontSize: 12, padding: '5px 8px' }}
                >
                  {t('ocr.destAppend')}
                </button>
              </div>
            </div>
          </div>

          {/* Extract Images Option */}
          <div
            style={{
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              padding: '10px 14px',
              background: 'var(--bg-input)',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-md)',
              cursor: isProcessing ? 'default' : 'pointer'
            }}
            onClick={() => !isProcessing && setExtractImages(!extractImages)}
          >
            <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
              <div style={{
                width: 28,
                height: 28,
                borderRadius: 'var(--radius-sm)',
                background: 'rgba(56, 189, 248, 0.12)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                flexShrink: 0
              }}>
                <Images size={16} color="var(--accent-primary)" />
              </div>
              <div>
                <div style={{ fontSize: 12, fontWeight: 600 }}>{t('ocr.extractImages')}</div>
                <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>{t('ocr.extractImagesDesc')}</div>
              </div>
            </div>
            <input
              type="checkbox"
              checked={extractImages}
              onChange={e => setExtractImages(e.target.checked)}
              disabled={isProcessing}
              style={{ cursor: 'pointer', width: 16, height: 16 }}
              onClick={e => e.stopPropagation()}
            />
          </div>

          {/* Image Placement Selection (Option B default: folderOnly) */}
          {extractImages && (
            <div style={{ marginTop: -4 }}>
              <label style={{ display: 'block', fontSize: 12, fontWeight: 600, marginBottom: 6 }}>
                {t('ocr.placement')}
              </label>
              <select
                className="select-field"
                value={imagePlacement}
                onChange={e => {
                  const val = e.target.value as any;
                  setImagePlacement(val);
                  localStorage.setItem('openmd_ocr_image_placement', val);
                }}
                disabled={isProcessing}
                style={{ width: '100%', fontSize: 12, padding: '7px 10px' }}
              >
                <option value="folderOnly">{t('ocr.placementFolderOnly')}</option>
                <option value="pageEnd">{t('ocr.placementPageEnd')}</option>
                <option value="docEnd">{t('ocr.placementDocEnd')}</option>
                <option value="inline">{t('ocr.placementInline')}</option>
              </select>
            </div>
          )}

          {/* Progress Indicator */}
          {isProcessing && (
            <div style={{
              background: 'var(--bg-elevated)',
              border: '1px solid var(--border-default)',
              borderRadius: 'var(--radius-md)',
              padding: '14px',
              display: 'flex',
              flexDirection: 'column',
              gap: 8
            }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, fontWeight: 500 }}>
                  <Loader2 size={14} className="spin" />
                  <span>
                    {progress && progress.totalPages > 0
                      ? t('ocr.progress', {
                          current: progress.page,
                          total: progress.totalPages,
                          percent: progress.percent
                        })
                      : t('ocr.processing')}
                  </span>
                </div>
                <span style={{ fontWeight: 600, color: 'var(--accent-primary)' }}>
                  %{progress?.percent || 0}
                </span>
              </div>

              {/* Progress Bar */}
              <div style={{
                height: 6,
                background: 'var(--bg-input)',
                borderRadius: 3,
                overflow: 'hidden'
              }}>
                <div style={{
                  height: '100%',
                  width: `${Math.max(5, progress?.percent || 0)}%`,
                  background: 'var(--accent-primary)',
                  transition: 'width 0.2s ease'
                }} />
              </div>
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'flex-end',
          gap: 10,
          padding: '14px 20px',
          borderTop: '1px solid var(--border-subtle)',
          background: 'var(--bg-elevated)'
        }}>
          <button
            className="btn-secondary"
            onClick={onClose}
            disabled={isProcessing}
            style={{ fontSize: 13, padding: '6px 14px' }}
          >
            {t('ocr.cancel')}
          </button>

          <button
            className="btn-primary"
            onClick={handleStartOCR}
            disabled={!pdfPath || isProcessing}
            style={{
              fontSize: 13,
              padding: '6px 16px',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              opacity: !pdfPath || isProcessing ? 0.6 : 1
            }}
          >
            {isProcessing ? (
              <>
                <Loader2 size={14} className="spin" />
                <span>{t('ocr.processing')}</span>
              </>
            ) : (
              <>
                <Sparkles size={14} />
                <span>{t('ocr.startBtn')}</span>
              </>
            )}
          </button>
        </div>
      </div>
    </div>
  );
};
