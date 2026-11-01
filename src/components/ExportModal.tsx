import React, { useState } from 'react';
import { X, FileCode, Printer, Check, Copy, FolderOpen } from 'lucide-react';
import { generateStandaloneHTML } from '../utils/markdownUtils';
import { useTranslation } from '../i18n/LanguageContext';

interface ExportModalProps {
  isOpen: boolean;
  onClose: () => void;
  documentTitle: string;
  markdownContent: string;
  defaultDir?: string | null;
}

export const ExportModal: React.FC<ExportModalProps> = ({
  isOpen,
  onClose,
  documentTitle,
  markdownContent,
  defaultDir
}) => {
  const { t } = useTranslation();
  const [copiedHTML, setCopiedHTML] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [lastSavedPath, setLastSavedPath] = useState<string | null>(null);
  const [copiedMD, setCopiedMD] = useState(false);

  if (!isOpen) return null;

  const handleExportHTML = async () => {
    try {
      setExporting(true);
      const previewEl = document.querySelector('.preview-content');
      const innerHTML = previewEl ? previewEl.innerHTML : `<pre>${markdownContent}</pre>`;
      const fullHTML = generateStandaloneHTML(documentTitle, innerHTML);

      if (window.electronAPI) {
        const defaultName = `${documentTitle.replace(/\.md$/, '')}.html`;
        const savedPath = await window.electronAPI.saveHTMLFile(fullHTML, defaultName, defaultDir);
        if (savedPath) {
          setLastSavedPath(savedPath);
          setMessage(t('export.htmlSuccess', { path: savedPath }));
        } else {
          setLastSavedPath(null);
          setMessage(t('export.htmlCanceled'));
        }
      } else {
        const blob = new Blob([fullHTML], { type: 'text/html;charset=utf-8' });
        const url = URL.createObjectURL(blob);
        const a = document.createElement('a');
        a.href = url;
        a.download = `${documentTitle.replace(/\.md$/, '')}.html`;
        a.click();
        URL.revokeObjectURL(url);
        setMessage(t('export.htmlDownloaded'));
      }
    } catch (err: any) {
      setMessage(t('export.error', { error: err.message }));
    } finally {
      setExporting(false);
    }
  };

  const handleExportPDF = async () => {
    try {
      setExporting(true);
      const previewEl = document.querySelector('.preview-content');
      const innerHTML = previewEl ? previewEl.innerHTML : `<pre>${markdownContent}</pre>`;
      const fullHTML = generateStandaloneHTML(documentTitle, innerHTML);

      if (window.electronAPI) {
        const defaultName = `${documentTitle.replace(/\.md$/, '')}.pdf`;
        await window.electronAPI.exportPDF(fullHTML, defaultName);
        setLastSavedPath(null);
        setMessage(t('export.pdfPrompt'));
      } else {
        window.print();
      }
    } catch (err: any) {
      setMessage(t('export.error', { error: err.message }));
    } finally {
      setExporting(false);
    }
  };

  const handleCopyRenderedHTML = () => {
    const previewEl = document.querySelector('.preview-content');
    if (previewEl) {
      navigator.clipboard.writeText(previewEl.innerHTML);
      setCopiedHTML(true);
      setTimeout(() => setCopiedHTML(false), 2000);
    }
  };

  const handleCopyMarkdown = () => {
    navigator.clipboard.writeText(markdownContent);
    setCopiedMD(true);
    setTimeout(() => setCopiedMD(false), 2000);
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div 
        className="palette-box" 
        style={{ width: 480 }}
        onClick={e => e.stopPropagation()}
      >
        <div style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '16px 20px',
          borderBottom: '1px solid var(--border-subtle)',
          background: 'var(--bg-elevated)'
        }}>
          <h3 style={{ fontSize: 16, fontWeight: 600 }}>{t('export.title')}</h3>
          <button 
            className="btn-icon" 
            onClick={onClose}
            style={{ width: 26, height: 26 }}
          >
            <X size={16} />
          </button>
        </div>

        <div style={{ padding: '20px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          {message && (
            <div style={{
              background: message.startsWith('Hata') || message.includes('error') ? 'var(--danger-subtle)' : 'var(--accent-subtle)',
              border: `1px solid ${message.startsWith('Hata') || message.includes('error') ? 'var(--danger-color)' : 'var(--accent-primary)'}`,
              borderRadius: 'var(--radius-sm)',
              padding: '10px 14px',
              fontSize: 13,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
              color: message.startsWith('Hata') || message.includes('error') ? 'var(--danger-color)' : 'var(--accent-primary)'
            }}>
              <div>{message}</div>
              {lastSavedPath && (
                <button
                  className="btn-secondary"
                  onClick={() => window.electronAPI?.showInFolder?.(lastSavedPath)}
                  style={{ alignSelf: 'flex-start', fontSize: 12, padding: '4px 10px', display: 'flex', alignItems: 'center', gap: 6 }}
                >
                  <FolderOpen size={13} />
                  <span>{t('export.showInFinder')}</span>
                </button>
              )}
            </div>
          )}

          {/* HTML Export Option */}
          <div style={{
            background: 'var(--bg-input)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-md)',
            padding: '14px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between'
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <div style={{
                width: 36,
                height: 36,
                borderRadius: 'var(--radius-sm)',
                background: 'rgba(56, 189, 248, 0.15)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center'
              }}>
                <FileCode size={20} color="#38bdf8" />
              </div>
              <div>
                <h4 style={{ fontSize: 14, fontWeight: 600 }}>HTML (.html)</h4>
                <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                  {t('export.htmlDesc')}
                </p>
              </div>
            </div>
            <button 
              className="btn-primary"
              onClick={handleExportHTML}
              disabled={exporting}
              style={{ whiteSpace: 'nowrap' }}
            >
              {t('export.htmlBtn')}
            </button>
          </div>

          {/* PDF Export Option */}
          <div style={{
            background: 'var(--bg-input)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-md)',
            padding: '14px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between'
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
              <div style={{
                width: 36,
                height: 36,
                borderRadius: 'var(--radius-sm)',
                background: 'rgba(239, 68, 68, 0.15)',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center'
              }}>
                <Printer size={20} color="#ef4444" />
              </div>
              <div>
                <h4 style={{ fontSize: 14, fontWeight: 600 }}>PDF (.pdf)</h4>
                <p style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                  {t('export.pdfDesc')}
                </p>
              </div>
            </div>
            <button 
              className="btn-primary"
              onClick={handleExportPDF}
              disabled={exporting}
              style={{ whiteSpace: 'nowrap' }}
            >
              {t('export.pdfBtn')}
            </button>
          </div>

          {/* Quick Copy Helpers */}
          <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
            <button 
              className="btn-secondary" 
              onClick={handleCopyRenderedHTML}
              style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6 }}
            >
              {copiedHTML ? <Check size={14} color="#10b981" /> : <Copy size={14} />}
              <span>{copiedHTML ? t('export.copied') : t('export.copyHtml')}</span>
            </button>
            <button 
              className="btn-secondary" 
              onClick={handleCopyMarkdown}
              style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6 }}
            >
              {copiedMD ? <Check size={14} color="#10b981" /> : <Copy size={14} />}
              <span>{copiedMD ? t('export.copied') : t('export.copyMd')}</span>
            </button>
          </div>
        </div>

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
          >
            {t('export.close')}
          </button>
        </div>
      </div>
    </div>
  );
};
