import React from 'react';
import { X, Plus, FileText } from 'lucide-react';
import { Tab } from '../types/electron';
import { useTranslation } from '../i18n/LanguageContext';

interface TabBarProps {
  tabs: Tab[];
  activeTabId: string;
  onSelectTab: (id: string) => void;
  onCloseTab: (id: string) => void;
  onNewTab: () => void;
}

export const TabBar: React.FC<TabBarProps> = ({
  tabs,
  activeTabId,
  onSelectTab,
  onCloseTab,
  onNewTab
}) => {
  const { t } = useTranslation();

  return (
    <div className="tab-bar-container">
      {tabs.map(tab => {
        const isActive = tab.id === activeTabId;
        return (
          <div
            key={tab.id}
            className={`tab-item ${isActive ? 'active' : ''}`}
            onClick={() => onSelectTab(tab.id)}
            title={tab.filePath || tab.title}
          >
            <FileText size={13} color={isActive ? '#38bdf8' : '#94a3b8'} />
            <span>{tab.title}</span>
            {tab.isModified && (
              <span 
                className="doc-modified-dot" 
                style={{ width: 6, height: 6 }} 
                title={t('tabbar.unsaved')}
              />
            )}
            <button
              className="tab-close-btn"
              onClick={(e) => {
                e.stopPropagation();
                onCloseTab(tab.id);
              }}
              title={t('tabbar.close')}
            >
              <X size={12} />
            </button>
          </div>
        );
      })}

      <button
        className="tab-new-btn"
        onClick={onNewTab}
        title={t('tabbar.newTab')}
      >
        <Plus size={15} />
      </button>
    </div>
  );
};
