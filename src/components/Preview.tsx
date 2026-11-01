import React, { useEffect, useRef, useState } from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import remarkMath from 'remark-math';
import rehypeKatex from 'rehype-katex';
import rehypeHighlight from 'rehype-highlight';
import mermaid from 'mermaid';
import { Copy, Check, Info, Lightbulb, AlertTriangle, ShieldAlert, AlertCircle } from 'lucide-react';
import { getDirname, resolveRelativeImagePath } from '../utils/markdownUtils';
import { useTranslation } from '../i18n/LanguageContext';

interface PreviewProps {
  content: string;
  theme: 'dark' | 'light' | 'auto';
  fontSize?: number;
  scrollRef?: React.RefObject<HTMLDivElement | null>;
  activeFilePath?: string;
  rootPath?: string;
  baseDir?: string;
  onToggleTask?: (taskIndex: number, checked: boolean) => void;
}

// Mermaid diagram renderer
const MermaidDiagram: React.FC<{ code: string; isDark: boolean }> = ({ code, isDark }) => {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);
  const [svgContent, setSvgContent] = useState<string>('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    mermaid.initialize({
      startOnLoad: false,
      theme: isDark ? 'dark' : 'default',
      securityLevel: 'loose',
      fontFamily: 'var(--font-ui)'
    });

    const id = `mermaid-${Math.random().toString(36).substring(2, 9)}`;
    mermaid.render(id, code)
      .then(result => {
        setSvgContent(result.svg);
        setError(null);
      })
      .catch(err => {
        console.warn('Mermaid render error:', err);
        setError(t('preview.diagramError'));
      });
  }, [code, isDark, t]);

  if (error) {
    return (
      <div className="mermaid-wrapper" style={{ color: 'var(--danger-color)', fontSize: 13 }}>
        {error}
      </div>
    );
  }

  return (
    <div 
      className="mermaid-wrapper" 
      ref={containerRef}
      dangerouslySetInnerHTML={{ __html: svgContent }}
    />
  );
};

// Helper to recursively extract plain text from React children (rehype-highlight token trees)
function extractCodeText(children: React.ReactNode): string {
  if (typeof children === 'string') return children;
  if (typeof children === 'number') return String(children);
  if (!children) return '';
  if (Array.isArray(children)) return children.map(extractCodeText).join('');
  if (React.isValidElement(children) && (children.props as any)?.children) {
    return extractCodeText((children.props as any).children);
  }
  return '';
}

// Code block with Copy button
interface CodeBlockProps {
  language?: string;
  className?: string;
  rawCode: string;
  isDark: boolean;
  children: React.ReactNode;
}

const CodeBlock: React.FC<CodeBlockProps> = ({ language, className, rawCode, isDark, children }) => {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    navigator.clipboard.writeText(rawCode);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  if (language === 'mermaid') {
    return <MermaidDiagram code={rawCode} isDark={isDark} />;
  }

  const codeClassName = className || (language ? `hljs language-${language}` : 'hljs');

  return (
    <div className="code-block-wrapper">
      <div className="code-block-header">
        <span>{language || 'text'}</span>
        <button className="copy-code-btn" onClick={handleCopy} title={t('preview.copy')}>
          {copied ? <Check size={13} color="#10b981" /> : <Copy size={13} />}
          <span>{copied ? t('preview.copied') : t('preview.copy')}</span>
        </button>
      </div>
      <pre>
        <code className={codeClassName}>
          {children}
        </code>
      </pre>
    </div>
  );
};

// Helper to remove [!TAG] tag from callout body
function cleanCalloutChildren(children: React.ReactNode, tag: string): React.ReactNode {
  let cleaned = false;
  return React.Children.map(children, (child) => {
    if (cleaned || !React.isValidElement(child)) return child;
    const childProps = (child.props as any) || {};
    if (!childProps.children) return child;

    const pChildren = React.Children.toArray(childProps.children);
    if (pChildren.length > 0 && typeof pChildren[0] === 'string') {
      const regex = new RegExp(`^\\s*\\[!${tag}\\]\\s*\\n?`, 'i');
      if (regex.test(pChildren[0])) {
        cleaned = true;
        const newFirst = pChildren[0].replace(regex, '');
        const newChildren = [newFirst, ...pChildren.slice(1)].filter(c => c !== '');
        if (newChildren.length === 0) {
          return null;
        }
        return React.cloneElement(child, { ...childProps, children: newChildren });
      }
    }
    return child;
  });
}

// In-memory cache for resolved base64 data URLs to eliminate flicker on re-renders
const base64ImageCache = new Map<string, string>();
const passthroughUrlTransform = (url: string) => url;

function getReadImageBase64Fn(): ((filePath: string, baseDir?: string) => Promise<string>) | undefined {
  if (typeof window !== 'undefined' && window.electronAPI?.readImageBase64) {
    return window.electronAPI.readImageBase64;
  }
  return undefined;
}

// Resilient image renderer that proactively resolves local assets via Tauri IPC into base64 data URLs
const PreviewImage: React.FC<{
  src?: string;
  alt?: string;
  effectiveBaseDir?: string;
  searchDirs?: string;
  onImageClick?: (src: string, alt: string) => void;
  [key: string]: any;
}> = ({ node: _node, src = '', alt = '', effectiveBaseDir, searchDirs, onImageClick, style, ...props }) => {
  const isRemoteOrData = /^(https?:\/\/|data:|blob:)/i.test(src);
  const lookupDir = searchDirs || effectiveBaseDir;
  const cacheKey = `${lookupDir || ''}::${src}`;

  const [currentSrc, setCurrentSrc] = useState<string | null>(() => {
    if (!src) return null;
    if (isRemoteOrData) return src;
    const cached = base64ImageCache.get(cacheKey);
    if (cached) return cached;
    if (getReadImageBase64Fn()) {
      return null;
    }
    return resolveRelativeImagePath(src, effectiveBaseDir);
  });

  useEffect(() => {
    if (!src) {
      setCurrentSrc(null);
      return;
    }
    if (/^(https?:\/\/|data:|blob:)/i.test(src)) {
      setCurrentSrc(src);
      return;
    }

    const dirArg = searchDirs || effectiveBaseDir;
    const key = `${dirArg || ''}::${src}`;
    const cached = base64ImageCache.get(key);
    if (cached) {
      setCurrentSrc(cached);
      return;
    }

    let cancelled = false;
    const readImageFn = getReadImageBase64Fn();

    if (readImageFn) {
      readImageFn(src, dirArg)
        .then((base64Src) => {
          if (!cancelled && base64Src) {
            base64ImageCache.set(key, base64Src);
            setCurrentSrc(base64Src);
          }
        })
        .catch((err) => {
          console.warn('Native image load failed for:', src, err);
          if (!cancelled) {
            if (effectiveBaseDir) {
              setCurrentSrc(resolveRelativeImagePath(src, effectiveBaseDir));
            } else {
              setCurrentSrc(null);
            }
          }
        });
    } else {
      setCurrentSrc(resolveRelativeImagePath(src, effectiveBaseDir));
    }

    return () => {
      cancelled = true;
    };
  }, [src, effectiveBaseDir, searchDirs]);

  if (!currentSrc) {
    return null;
  }

  return (
    <img
      src={currentSrc}
      alt={alt}
      onClick={() => onImageClick?.(currentSrc, alt || '')}
      onError={() => {
        if (!currentSrc.startsWith('data:')) {
          setCurrentSrc(null);
        }
      }}
      title="Büyütmek için tıklayın"
      style={{
        maxWidth: '100%',
        maxHeight: '360px',
        width: 'auto',
        height: 'auto',
        objectFit: 'contain',
        borderRadius: 'var(--radius-sm)',
        boxShadow: '0 2px 8px rgba(0, 0, 0, 0.08)',
        margin: '12px auto',
        display: 'block',
        cursor: 'zoom-in',
        ...style
      }}
      {...props}
    />
  );
};

export const Preview: React.FC<PreviewProps> = React.memo(({ 
  content, 
  theme, 
  fontSize = 14,
  scrollRef,
  activeFilePath,
  rootPath,
  baseDir,
  onToggleTask 
}) => {
  const isDark = theme === 'dark' || (theme === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  const [lightboxImg, setLightboxImg] = useState<{ src: string; alt: string } | null>(null);
  let taskCounter = 0;

  return (
    <div className="preview-scroll-container" ref={scrollRef}>
      <div className="preview-content" style={{ fontSize: `${fontSize}px` }}>
        <ReactMarkdown
          urlTransform={passthroughUrlTransform}
          remarkPlugins={[remarkGfm, remarkMath]}
          rehypePlugins={[rehypeKatex, rehypeHighlight]}
          components={{
            // Interactive Task Checkboxes
            input({ node, checked, disabled, ...props }: any) {
              if (props.type === 'checkbox') {
                const currentIndex = taskCounter++;
                return (
                  <input
                    type="checkbox"
                    checked={Boolean(checked)}
                    onChange={(e) => {
                      if (onToggleTask) {
                        onToggleTask(currentIndex, e.target.checked);
                      }
                    }}
                    style={{ cursor: onToggleTask ? 'pointer' : 'default', marginRight: 8 }}
                    {...props}
                  />
                );
              }
              return <input {...props} />;
            },

            // Custom code block renderer (wrapped inside pre)
            pre({ children }: any) {
              const codeEl = React.isValidElement(children) ? (children as React.ReactElement<any>) : null;
              if (codeEl) {
                const className = codeEl.props?.className || '';
                const match = /language-(\w+)/.exec(className);
                const lang = match ? match[1] : undefined;
                const codeChildren = codeEl.props?.children;
                const rawCode = extractCodeText(codeChildren);

                return (
                  <CodeBlock
                    language={lang}
                    className={className}
                    rawCode={rawCode}
                    isDark={isDark}
                  >
                    {codeChildren}
                  </CodeBlock>
                );
              }

              const rawCode = extractCodeText(children);
              return (
                <CodeBlock
                  rawCode={rawCode}
                  isDark={isDark}
                >
                  {children}
                </CodeBlock>
              );
            },

            // Standard inline code tags
            code({ node, className, children, ...props }: any) {
              return (
                <code className={className} {...props}>
                  {children}
                </code>
              );
            },

            // Responsive image renderer with local asset resolution & auto-fallback
            img({ node, src, alt, ...props }: any) {
              const activeDir = activeFilePath ? getDirname(activeFilePath) : undefined;
              const effectiveBaseDir = activeDir || baseDir || rootPath;
              const searchDirs = [activeDir, baseDir, rootPath].filter(Boolean).join('\n') || undefined;
              return (
                <PreviewImage
                  src={src}
                  alt={alt}
                  effectiveBaseDir={effectiveBaseDir}
                  searchDirs={searchDirs}
                  onImageClick={(imgSrc, imgAlt) => setLightboxImg({ src: imgSrc, alt: imgAlt })}
                  {...props}
                />
              );
            },

            // Custom blockquote / GitHub alerts with tag cleanup
            blockquote({ children }) {
              const textContent = React.Children.toArray(children)
                .map(child => (typeof child === 'object' && 'props' in (child as any) ? (child as any).props.children : ''))
                .join(' ');

              if (typeof textContent === 'string') {
                const match = textContent.match(/\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]/i);
                if (match) {
                  const type = match[1].toUpperCase();
                  switch (type) {
                    case 'NOTE':
                      return (
                        <div className="callout callout-note">
                          <div className="callout-header">
                            <Info size={16} />
                            <span>Not</span>
                          </div>
                          <div>{cleanCalloutChildren(children, 'NOTE')}</div>
                        </div>
                      );
                    case 'TIP':
                      return (
                        <div className="callout callout-tip">
                          <div className="callout-header">
                            <Lightbulb size={16} />
                            <span>İpucu</span>
                          </div>
                          <div>{cleanCalloutChildren(children, 'TIP')}</div>
                        </div>
                      );
                    case 'IMPORTANT':
                      return (
                        <div className="callout callout-important">
                          <div className="callout-header">
                            <AlertCircle size={16} />
                            <span>Önemli</span>
                          </div>
                          <div>{cleanCalloutChildren(children, 'IMPORTANT')}</div>
                        </div>
                      );
                    case 'WARNING':
                      return (
                        <div className="callout callout-warning">
                          <div className="callout-header">
                            <AlertTriangle size={16} />
                            <span>Uyarı</span>
                          </div>
                          <div>{cleanCalloutChildren(children, 'WARNING')}</div>
                        </div>
                      );
                    case 'CAUTION':
                      return (
                        <div className="callout callout-caution">
                          <div className="callout-header">
                            <ShieldAlert size={16} />
                            <span>Dikkat</span>
                          </div>
                          <div>{cleanCalloutChildren(children, 'CAUTION')}</div>
                        </div>
                      );
                  }
                }
              }

              return <blockquote>{children}</blockquote>;
            },

            // Table wrapper
            table({ children }) {
              return (
                <div style={{ overflowX: 'auto', margin: '1.4em 0' }}>
                  <table>{children}</table>
                </div>
              );
            }
          }}
        >
          {content}
        </ReactMarkdown>
      </div>

      {lightboxImg && (
        <div
          role="dialog"
          aria-modal="true"
          onClick={() => setLightboxImg(null)}
          style={{
            position: 'fixed',
            inset: 0,
            zIndex: 9999,
            backgroundColor: 'rgba(0, 0, 0, 0.78)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            padding: '24px',
            backdropFilter: 'blur(5px)',
            cursor: 'zoom-out'
          }}
        >
          <div
            onClick={(e) => e.stopPropagation()}
            style={{
              position: 'relative',
              maxWidth: '90vw',
              maxHeight: '90vh',
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center'
            }}
          >
            <img
              src={lightboxImg.src}
              alt={lightboxImg.alt}
              style={{
                maxWidth: '100%',
                maxHeight: '82vh',
                objectFit: 'contain',
                borderRadius: '8px',
                boxShadow: '0 8px 32px rgba(0, 0, 0, 0.5)'
              }}
            />
            {lightboxImg.alt && (
              <span
                style={{
                  marginTop: '10px',
                  color: '#f8fafc',
                  fontSize: '13px',
                  background: 'rgba(15, 23, 42, 0.75)',
                  padding: '4px 12px',
                  borderRadius: '12px',
                  letterSpacing: '0.01em'
                }}
              >
                {lightboxImg.alt}
              </span>
            )}
            <button
              type="button"
              onClick={() => setLightboxImg(null)}
              style={{
                position: 'absolute',
                top: -12,
                right: -12,
                background: '#334155',
                color: '#ffffff',
                border: '1px solid rgba(255, 255, 255, 0.2)',
                borderRadius: '50%',
                width: 28,
                height: 28,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                cursor: 'pointer',
                fontSize: 16,
                lineHeight: 1,
                boxShadow: '0 2px 8px rgba(0, 0, 0, 0.3)'
              }}
              title="Kapat"
            >
              ×
            </button>
          </div>
        </div>
      )}
    </div>
  );
});
