import { useEffect } from 'react';
import AppLayout from './components/Layout';
import { useConnectionStore } from './stores/connectionStore';
import { useThemeStore } from './stores/themeStore';
import { useClipboardStore } from './stores/clipboardStore';
import { ToastContainer } from './components/ui/toast';
import CreateResourceDialog from './components/CreateResourceDialog';
import ConfirmDialog from './components/ConfirmDialog';
import i18n from './i18n';
import '@/lib/plugins'; // Initialize plugin registry
import './styles/global.css';

function App() {
  const { setConnections } = useConnectionStore();
  const initTheme = useThemeStore((s) => s.initTheme);

  useEffect(() => {
    const cleanupTheme = initTheme();
    const savedLang = localStorage.getItem('astesia_language');
    if (savedLang) i18n.changeLanguage(savedLang);
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

  // Disable Tauri default context menu globally
  // Radix ContextMenu intercepts right-click at higher priority, so custom
  // context menus still work. This only suppresses the native menu elsewhere.
  useEffect(() => {
    const handleContextMenu = (e: MouseEvent) => {
      e.preventDefault();
    };
    document.addEventListener('contextmenu', handleContextMenu);
    return () => document.removeEventListener('contextmenu', handleContextMenu);
  }, []);

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

  return (
    <>
      <AppLayout />
      <ToastContainer />
      <CreateResourceDialog />
      <ConfirmDialog />
    </>
  );
}

export default App;
