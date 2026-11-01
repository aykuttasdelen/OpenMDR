import React, { createContext, useContext, useState, useEffect, ReactNode, useCallback } from 'react';
import { Language, getTranslation } from './translations';

interface LanguageContextType {
  language: Language;
  setLanguage: (lang: Language) => void;
  t: (key: string, params?: Record<string, string | number>) => string;
}

const LanguageContext = createContext<LanguageContextType>({
  language: 'tr',
  setLanguage: () => {},
  t: (key, params) => getTranslation('tr', key, params)
});

export const LANGUAGE_STORAGE_KEY = 'openmd-language';

export const LanguageProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
  const [language, setLanguageState] = useState<Language>(() => {
    const saved = localStorage.getItem(LANGUAGE_STORAGE_KEY);
    if (saved === 'en' || saved === 'tr') {
      return saved;
    }
    // Default to system language if English, else Turkish
    const navLang = navigator.language.toLowerCase();
    return navLang.startsWith('tr') ? 'tr' : 'en';
  });

  const setLanguage = useCallback((lang: Language) => {
    setLanguageState(lang);
    localStorage.setItem(LANGUAGE_STORAGE_KEY, lang);
    if (window.electronAPI && (window.electronAPI as any).setLanguage) {
      (window.electronAPI as any).setLanguage(lang);
    }
  }, []);

  useEffect(() => {
    // Notify electron on startup
    if (window.electronAPI && (window.electronAPI as any).setLanguage) {
      (window.electronAPI as any).setLanguage(language);
    }
  }, [language]);

  const t = useCallback((key: string, params?: Record<string, string | number>) => {
    return getTranslation(language, key, params);
  }, [language]);

  return (
    <LanguageContext.Provider value={{ language, setLanguage, t }}>
      {children}
    </LanguageContext.Provider>
  );
};

export const useTranslation = () => useContext(LanguageContext);
