import { useEffect } from 'react';
import AppLayout from './components/Layout';
import { useConnectionStore } from './stores/connectionStore';
import { useThemeStore } from './stores/themeStore';
import { useClipboardStore } from './stores/clipboardStore';
import './i18n';
import './styles/global.css';

function App() {
  const { setConnections } = useConnectionStore();
  const initTheme = useThemeStore((s) => s.initTheme);

  useEffect(() => {
    const cleanupTheme = initTheme();
    const saved = localStorage.getItem('astesia_connections');
    if (saved) {
      try {
        setConnections(JSON.parse(saved));
      } catch {
        // ignore
      }
    }

    const unsubscribe = useConnectionStore.subscribe((state) => {
      localStorage.setItem(
        'astesia_connections',
        JSON.stringify(state.connections)
      );
    });
    return () => {
      unsubscribe();
      cleanupTheme();
    };
  }, [setConnections, initTheme]);

  // Global keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Ctrl+/- zoom
      if ((e.ctrlKey || e.metaKey) && (e.key === '=' || e.key === '+')) {
        e.preventDefault();
        document.documentElement.style.fontSize =
          Math.min(24, parseFloat(getComputedStyle(document.documentElement).fontSize) + 1) + 'px';
      }
      if ((e.ctrlKey || e.metaKey) && e.key === '-') {
        e.preventDefault();
        document.documentElement.style.fontSize =
          Math.max(10, parseFloat(getComputedStyle(document.documentElement).fontSize) - 1) + 'px';
      }
      if ((e.ctrlKey || e.metaKey) && e.key === '0') {
        e.preventDefault();
        document.documentElement.style.fontSize = '';
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  return <AppLayout />;
}

export default App;
