import React from 'react';
import { 
  Bold, 
  Italic, 
  Strikethrough, 
  Heading1, 
  Heading2, 
  Heading3, 
  Code, 
  Terminal, 
  Link2, 
  Quote, 
  List, 
  ListOrdered, 
  CheckSquare, 
  Table, 
  Sigma,
  Info
} from 'lucide-react';
import { useTranslation } from '../i18n/LanguageContext';

interface FormattingBarProps {
  onInsert: (prefix: string, suffix?: string, defaultText?: string) => void;
}

export const FormattingBar: React.FC<FormattingBarProps> = ({ onInsert }) => {
  const { t } = useTranslation();

  return (
    <div className="formatting-bar">
      <button 
        className="format-btn" 
        onClick={() => onInsert('**', '**', 'bold text')} 
        title={t('format.bold')}
      >
        <Bold size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('*', '*', 'italic text')} 
        title={t('format.italic')}
      >
        <Italic size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('~~', '~~', 'strikethrough')} 
        title={t('format.strikethrough')}
      >
        <Strikethrough size={15} />
      </button>

      <div className="format-divider" />

      <button 
        className="format-btn" 
        onClick={() => onInsert('# ', '', 'Heading 1')} 
        title={t('format.h1')}
      >
        <Heading1 size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('## ', '', 'Heading 2')} 
        title={t('format.h2')}
      >
        <Heading2 size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('### ', '', 'Heading 3')} 
        title={t('format.h3')}
      >
        <Heading3 size={15} />
      </button>

      <div className="format-divider" />

      <button 
        className="format-btn" 
        onClick={() => onInsert('`', '`', 'code')} 
        title={t('format.inlineCode')}
      >
        <Code size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('```typescript\n', '\n```', '// code here')} 
        title={t('format.codeBlock')}
      >
        <Terminal size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('[', '](https://example.com)', 'link text')} 
        title={t('format.link')}
      >
        <Link2 size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('> ', '', 'Quote text')} 
        title={t('format.quote')}
      >
        <Quote size={15} />
      </button>

      <div className="format-divider" />

      <button 
        className="format-btn" 
        onClick={() => onInsert('- ', '', 'Item')} 
        title={t('format.bulletList')}
      >
        <List size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('1. ', '', 'Item')} 
        title={t('format.numberedList')}
      >
        <ListOrdered size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('- [ ] ', '', 'Task')} 
        title={t('format.taskList')}
      >
        <CheckSquare size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('| Title 1 | Title 2 |\n| :--- | :--- |\n| Data 1 | Data 2 |\n', '')} 
        title={t('format.table')}
      >
        <Table size={15} />
      </button>

      <div className="format-divider" />

      <button 
        className="format-btn" 
        onClick={() => onInsert('$$\n', '\n$$', 'E = mc^2')} 
        title={t('format.math')}
      >
        <Sigma size={15} />
      </button>
      <button 
        className="format-btn" 
        onClick={() => onInsert('> [!NOTE]\n> ', '', 'Note content')} 
        title={t('format.callout')}
      >
        <Info size={15} />
      </button>
    </div>
  );
};
