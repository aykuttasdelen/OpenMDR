import React, { useEffect, useRef, useImperativeHandle, forwardRef, useCallback } from 'react';
import { EditorState, Compartment } from '@codemirror/state';
import { 
  EditorView, 
  lineNumbers, 
  highlightActiveLine, 
  highlightActiveLineGutter, 
  keymap, 
  drawSelection 
} from '@codemirror/view';
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { syntaxHighlighting, defaultHighlightStyle } from '@codemirror/language';
import { oneDark } from '@codemirror/theme-one-dark';
import { openSearchPanel, searchKeymap } from '@codemirror/search';

export interface EditorHandle {
  insertText: (prefix: string, suffix?: string, defaultText?: string) => void;
  scrollToLine: (lineNumber: number) => void;
  openFind: () => void;
  focus: () => void;
}

interface EditorProps {
  docId: string;
  value: string;
  theme: 'dark' | 'light' | 'auto';
  fontSize?: number;
  baseDir?: string;
  onPasteImage?: (file: File) => Promise<string | null>;
  onImageSaved?: (savedPath: string) => void;
  onChange: (value: string, docId?: string) => void;
  onScroll?: (scrollPercentage: number) => void;
  onCursorChange?: (line: number, col: number) => void;
}

export const Editor = forwardRef<EditorHandle, EditorProps>(({
  docId,
  value,
  theme,
  fontSize = 14,
  baseDir,
  onPasteImage,
  onImageSaved,
  onChange,
  onScroll,
  onCursorChange
}, ref) => {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const themeCompartment = useRef(new Compartment());
  const fontSizeCompartment = useRef(new Compartment());
  const isUpdatingFromProp = useRef(false);

  // Stale closure guards
  const onChangeRef = useRef(onChange);
  const onScrollRef = useRef(onScroll);
  const onCursorChangeRef = useRef(onCursorChange);
  const currentDocIdRef = useRef(docId);
  const themeRef = useRef(theme);
  const fontSizeRef = useRef(fontSize);
  const baseDirRef = useRef(baseDir);
  const onPasteImageRef = useRef(onPasteImage);
  const onImageSavedRef = useRef(onImageSaved);

  useEffect(() => {
    onChangeRef.current = onChange;
  }, [onChange]);

  useEffect(() => {
    onScrollRef.current = onScroll;
  }, [onScroll]);

  useEffect(() => {
    onCursorChangeRef.current = onCursorChange;
  }, [onCursorChange]);

  useEffect(() => {
    themeRef.current = theme;
  }, [theme]);

  useEffect(() => {
    fontSizeRef.current = fontSize;
  }, [fontSize]);

  useEffect(() => {
    baseDirRef.current = baseDir;
  }, [baseDir]);

  useEffect(() => {
    onPasteImageRef.current = onPasteImage;
  }, [onPasteImage]);

  useEffect(() => {
    onImageSavedRef.current = onImageSaved;
  }, [onImageSaved]);

  // Tab State Isolation: Map of docId -> EditorState
  const tabStatesRef = useRef<Map<string, EditorState>>(new Map());

  // Expose imperative methods to parent
  useImperativeHandle(ref, () => ({
    insertText: (prefix: string, suffix = '', defaultText = '') => {
      const view = viewRef.current;
      if (!view) return;

      const state = view.state;
      const range = state.selection.main;
      const selected = state.sliceDoc(range.from, range.to) || defaultText;
      const replacement = `${prefix}${selected}${suffix}`;

      view.dispatch({
        changes: {
          from: range.from,
          to: range.to,
          insert: replacement
        },
        selection: {
          anchor: range.from + prefix.length,
          head: range.from + prefix.length + selected.length
        }
      });
      view.focus();
    },

    scrollToLine: (lineNumber: number) => {
      const view = viewRef.current;
      if (!view) return;
      try {
        const line = view.state.doc.line(Math.min(lineNumber, view.state.doc.lines));
        view.dispatch({
          selection: { anchor: line.from },
          scrollIntoView: true
        });
        view.focus();
      } catch (err) {
        console.error('Failed to scroll to line:', err);
      }
    },

    openFind: () => {
      const view = viewRef.current;
      if (!view) return;
      openSearchPanel(view);
    },

    focus: () => {
      viewRef.current?.focus();
    }
  }));

  const baseTheme = EditorView.theme({
    '&': {
      height: '100%'
    },
    '.cm-content': {
      padding: '16px 20px',
      caretColor: '#3b82f6'
    },
    '.cm-scroller': {
      fontFamily: 'var(--font-editor)',
      lineHeight: '1.6'
    },
    '.cm-line': {
      padding: '0 4px'
    }
  });

  const getFontSizeTheme = (size: number) => {
    return EditorView.theme({
      '&': {
        fontSize: `${size}px`
      }
    });
  };

  const createExtensions = (currentTheme: 'dark' | 'light' | 'auto', currentSize: number) => {
    const isDarkMode = currentTheme === 'dark' || (currentTheme === 'auto' && (typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches));

    const updateListener = EditorView.updateListener.of((update) => {
      if (update.docChanged && !isUpdatingFromProp.current) {
        onChangeRef.current(update.state.doc.toString(), currentDocIdRef.current);
      }

      if (update.selectionSet && onCursorChangeRef.current) {
        const pos = update.state.selection.main.head;
        const line = update.state.doc.lineAt(pos);
        onCursorChangeRef.current(line.number, pos - line.from + 1);
      }
    });

    const domHandlers = EditorView.domEventHandlers({
      scroll: (event) => {
        if (!onScrollRef.current) return;
        const target = event.target as HTMLElement;
        const maxScroll = target.scrollHeight - target.clientHeight;
        if (maxScroll > 0) {
          const percentage = target.scrollTop / maxScroll;
          onScrollRef.current(percentage);
        }
      },
      paste: (event, view) => {
        // Smart Paste: 1) URL paste over selection
        const clipboardText = event.clipboardData?.getData('text/plain');
        if (clipboardText) {
          const trimmedUrl = clipboardText.trim();
          const urlPattern = /^https?:\/\/[^\s]+$/i;
          if (urlPattern.test(trimmedUrl)) {
            const range = view.state.selection.main;
            if (!range.empty) {
              const selectedText = view.state.sliceDoc(range.from, range.to);
              // Avoid double-wrapping if already a markdown link
              if (!/^\[.*\]\(.*\)$/.test(selectedText)) {
                event.preventDefault();
                const replacement = `[${selectedText}](${trimmedUrl})`;
                view.dispatch({
                  changes: {
                    from: range.from,
                    to: range.to,
                    insert: replacement
                  },
                  selection: { anchor: range.from + replacement.length }
                });
                return true;
              }
            }
          }
        }

        // Smart Paste: 2) Image paste
        const items = event.clipboardData?.items;
        const files = event.clipboardData?.files;
        let imageFile: File | null = null;

        if (items) {
          for (let i = 0; i < items.length; i++) {
            const item = items[i];
            if (item.type.startsWith('image/')) {
              const f = item.getAsFile();
              if (f) {
                imageFile = f;
                break;
              }
            }
          }
        }

        if (!imageFile && files && files.length > 0) {
          for (let i = 0; i < files.length; i++) {
            const f = files[i];
            if (f.type.startsWith('image/') || /\.(png|jpe?g|webp|gif|svg)$/i.test(f.name)) {
              imageFile = f;
              break;
            }
          }
        }

        if (imageFile) {
          event.preventDefault();
          const file = imageFile;
          const mimeType = file.type || 'image/png';
          const reader = new FileReader();
          reader.onload = async () => {
            const currentView = viewRef.current;
            if (!currentView || currentView !== view) return;
            const dataUrl = reader.result as string;
            let imagePath = dataUrl;

            if (onPasteImageRef.current) {
              try {
                const customPath = await onPasteImageRef.current(file);
                if (customPath) {
                  imagePath = customPath;
                  onImageSavedRef.current?.(customPath);
                }
              } catch (err) {
                console.error('Custom onPasteImage error:', err);
              }
            } else if (window.electronAPI?.saveImage && baseDirRef.current) {
              try {
                const savedPath = await window.electronAPI.saveImage(
                  baseDirRef.current,
                  dataUrl,
                  mimeType
                );
                if (savedPath) {
                  imagePath = savedPath;
                  onImageSavedRef.current?.(savedPath);
                }
              } catch (err) {
                console.error('Failed to save image in Electron:', err);
              }
            }

            if (!viewRef.current || viewRef.current !== view) return;
            const range = currentView.state.selection.main;
            const replacement = `![Görsel](${imagePath})`;
            currentView.dispatch({
              changes: {
                from: range.from,
                to: range.to,
                insert: replacement
              },
              selection: { anchor: range.from + replacement.length }
            });
            currentView.focus();
          };
          reader.readAsDataURL(file);
          return true;
        }

        return false;
      }
    });

    return [
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      drawSelection(),
      history(),
      markdown({ codeLanguages: languages }),
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      keymap.of([
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        indentWithTab
      ]),
      themeCompartment.current.of(isDarkMode ? oneDark : []),
      fontSizeCompartment.current.of(getFontSizeTheme(currentSize)),
      baseTheme,
      updateListener,
      domHandlers,
      EditorView.lineWrapping
    ];
  };

  // Initialize CodeMirror 6 View
  useEffect(() => {
    if (!containerRef.current) return;

    const state = EditorState.create({
      doc: value,
      extensions: createExtensions(theme, fontSize)
    });

    tabStatesRef.current.set(docId, state);
    currentDocIdRef.current = docId;

    const view = new EditorView({
      state,
      parent: containerRef.current
    });

    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
  }, []);

  // Handle Tab Switch (docId change) with isolated EditorState & compartment syncing
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    if (currentDocIdRef.current === docId) {
      // Same tab: check if value was updated externally
      const currentDoc = view.state.doc.toString();
      if (value !== currentDoc) {
        isUpdatingFromProp.current = true;
        view.dispatch({
          changes: {
            from: 0,
            to: currentDoc.length,
            insert: value
          }
        });
        isUpdatingFromProp.current = false;
      }
      return;
    }

    // Save previous tab state
    if (currentDocIdRef.current) {
      tabStatesRef.current.set(currentDocIdRef.current, view.state);
    }

    // Switch to new tab state
    currentDocIdRef.current = docId;
    isUpdatingFromProp.current = true;

    const isDarkMode = theme === 'dark' || (theme === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    const reconfigureEffects = [
      themeCompartment.current.reconfigure(isDarkMode ? oneDark : []),
      fontSizeCompartment.current.reconfigure(getFontSizeTheme(fontSize))
    ];

    const existingState = tabStatesRef.current.get(docId);
    if (existingState) {
      view.setState(existingState);
      // Ensure theme, font size, and latest content are applied
      if (existingState.doc.toString() !== value) {
        view.dispatch({
          changes: {
            from: 0,
            to: existingState.doc.length,
            insert: value
          },
          effects: reconfigureEffects
        });
      } else {
        view.dispatch({ effects: reconfigureEffects });
      }
    } else {
      const newState = EditorState.create({
        doc: value,
        extensions: createExtensions(theme, fontSize)
      });
      tabStatesRef.current.set(docId, newState);
      view.setState(newState);
    }

    // Prune tab cache if it grows too large
    if (tabStatesRef.current.size > 80) {
      const keysToKeep = [docId];
      for (const key of tabStatesRef.current.keys()) {
        if (!keysToKeep.includes(key)) {
          tabStatesRef.current.delete(key);
          if (tabStatesRef.current.size <= 40) break;
        }
      }
    }

    isUpdatingFromProp.current = false;
  }, [docId, value, theme, fontSize]);

  // Update theme when changed
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    const isDarkMode = theme === 'dark' || (theme === 'auto' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    view.dispatch({
      effects: themeCompartment.current.reconfigure(isDarkMode ? oneDark : [])
    });
  }, [theme]);

  // Update font size when changed
  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    view.dispatch({
      effects: fontSizeCompartment.current.reconfigure(getFontSizeTheme(fontSize))
    });
  }, [fontSize]);

  return <div ref={containerRef} style={{ height: '100%', width: '100%', overflow: 'hidden' }} />;
});
