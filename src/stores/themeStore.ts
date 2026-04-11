import { create } from 'zustand';

type ThemeMode = 'light' | 'dark' | 'system';
type ResolvedTheme = 'light' | 'dark';

const STORAGE_KEY = 'astesia_theme';
const MEDIA_QUERY = '(prefers-color-scheme: dark)';

interface ThemeStore {
  theme: ThemeMode;
  resolvedTheme: ResolvedTheme;
  setTheme: (mode: ThemeMode) => void;
  initTheme: () => () => void;
}

function applyThemeClass(resolved: ResolvedTheme) {
  const root = document.documentElement;
  if (resolved === 'dark') {
    root.classList.add('dark');
  } else {
    root.classList.remove('dark');
  }
}

function resolveTheme(mode: ThemeMode): ResolvedTheme {
  if (mode === 'system') {
    return window.matchMedia(MEDIA_QUERY).matches ? 'dark' : 'light';
  }
  return mode;
}

export const useThemeStore = create<ThemeStore>((set, get) => ({
  theme: 'system',
  resolvedTheme: 'light',

  setTheme: (mode) => {
    const resolved = resolveTheme(mode);
    localStorage.setItem(STORAGE_KEY, mode);
    applyThemeClass(resolved);
    set({ theme: mode, resolvedTheme: resolved });
  },

  initTheme: () => {
    const saved = (localStorage.getItem(STORAGE_KEY) as ThemeMode) || 'system';
    const resolved = resolveTheme(saved);
    applyThemeClass(resolved);
    set({ theme: saved, resolvedTheme: resolved });

    const mediaQuery = window.matchMedia(MEDIA_QUERY);
    const handleChange = () => {
      const current = get().theme;
      if (current === 'system') {
        const newResolved = mediaQuery.matches ? 'dark' : 'light';
        applyThemeClass(newResolved);
        set({ resolvedTheme: newResolved });
      }
    };

    mediaQuery.addEventListener('change', handleChange);
    return () => mediaQuery.removeEventListener('change', handleChange);
  },
}));
