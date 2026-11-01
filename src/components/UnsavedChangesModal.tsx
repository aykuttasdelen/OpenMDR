import React from 'react';
import { AlertTriangle, FileText, Save, Trash2, X } from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';
import { Tab } from '../types/electron';

interface UnsavedChangesModalProps {
  isOpen: boolean;
  unsavedTabs: Tab[];
  onSaveAllAndClose: () => Promise<void>;
  onDiscardAndClose: () => void;
  onCancel: () => void;
}

export const UnsavedChangesModal: React.FC<UnsavedChangesModalProps> = ({
  isOpen,
  unsavedTabs,
  onSaveAllAndClose,
  onDiscardAndClose,
  onCancel
}) => {
  const { t } = useTranslation();

  if (!isOpen) return null;

  return (
    <div className="modal-overlay" onClick={onCancel} style={{ zIndex: 1200 }}>
      <div 
        className="palette-box" 
        style={{ width: 520, maxWidth: '90vw' }}
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
              width: 32,
              height: 32,
              borderRadius: '50%',
              background: 'rgba(245, 158, 11, 0.15)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center'
            }}>
              <AlertTriangle size={18} color="#f59e0b" />
            </div>
            <h3 style={{ fontSize: 16, fontWeight: 600 }}>{t('app.unsavedWorkspaceTitle')}</h3>
          </div>
          <button 
            className="btn-icon" 
            onClick={onCancel}
            style={{ width: 26, height: 26 }}
          >
            <X size={16} />
          </button>
        </div>

        {/* Content */}
        <div style={{ padding: '20px' }}>
          <p style={{ fontSize: 14, color: 'var(--text-secondary)', lineHeight: 1.5, marginBottom: 16 }}>
            {t('app.unsavedWorkspaceMsg', { count: unsavedTabs.length })}
          </p>

          <div style={{
            background: 'var(--bg-input)',
            border: '1px solid var(--border-default)',
            borderRadius: 'var(--radius-md)',
            maxHeight: 160,
            overflowY: 'auto',
            padding: '8px 12px'
          }}>
            {unsavedTabs.map(tab => (
              <div 
                key={tab.id}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  padding: '6px 4px',
                  fontSize: 13,
                  color: 'var(--text-primary)',
                  borderBottom: '1px solid var(--border-subtle)'
                }}
              >
                <FileText size={14} color="#38bdf8" />
                <span style={{ fontWeight: 500 }}>{tab.title}</span>
                {tab.filePath && (
                  <span style={{ fontSize: 11, color: 'var(--text-muted)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    ({tab.filePath})
                  </span>
                )}
              </div>
            ))}
          </div>
        </div>

        {/* Footer actions */}
        <div style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          padding: '14px 20px',
          borderTop: '1px solid var(--border-subtle)',
          background: 'var(--bg-elevated)',
          gap: 10
        }}>
          <button
            className="btn-secondary"
            onClick={onCancel}
            style={{ padding: '7px 14px' }}
          >
            {t('app.cancel')}
          </button>

          <div style={{ display: 'flex', gap: 8 }}>
            <button
              className="btn-secondary"
              onClick={onDiscardAndClose}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 6,
                padding: '7px 14px',
                color: 'var(--danger-color)',
                borderColor: 'var(--danger-subtle)'
              }}
            >
              <Trash2 size={14} />
              <span>{t('app.discardAndClose')}</span>
            </button>

            <button
              className="btn-primary"
              onClick={onSaveAllAndClose}
              style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '7px 16px' }}
            >
              <Save size={14} />
              <span>{t('app.saveAllAndClose')}</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};
