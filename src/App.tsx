import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import AppLayout from './components/Layout';
import {
  sharedProfileToConfig,
  useConnectionStore,
} from './stores/connectionStore';
import type {
  ConnectionConfig,
  ConnectionProfilesSnapshot,
  ConnectionRepositoryError,
  LegacyMigrationResult,
  McpConnectionsSnapshot,
} from './types/database';
import { useThemeStore } from './stores/themeStore';
import { ToastContainer } from './components/ui/toast';
import CreateResourceDialog from './components/CreateResourceDialog';
import ConfirmDialog from './components/ConfirmDialog';
import UpdateDialog from './components/UpdateDialog';
import ConnectionMigrationGate, {
  type MigrationFailure,
} from './components/ConnectionMigrationGate';
import { useUpdateStore } from './stores/updateStore';
import i18n from './i18n';
import '@/lib/plugins'; // Initialize plugin registry
import './styles/global.css';

type MigrationPhase = 'checking' | 'required' | 'migrating' | 'ready';

const readLegacyConnections = (): {
  raw: string | null;
  connections: ConnectionConfig[] | null;
  failure: MigrationFailure | null;
} => {
  const raw = localStorage.getItem('astesia_connections');
  if (raw === null) return { raw, connections: [], failure: null };
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) {
      return {
        raw,
        connections: null,
        failure: {
          code: 'migration_incomplete',
          message: '旧版连接数据格式无效，Astesia 未删除任何数据。',
          remediation: '请先备份 localStorage 中的 astesia_connections，再修复其 JSON 格式。',
        },
      };
    }
    return {
      raw,
      connections: (parsed as unknown[])
        .filter((connection) =>
          (connection as { source?: unknown } | null)?.source !== 'mcp_http'
        )
        .map((connection) => connection as ConnectionConfig),
      failure: null,
    };
  } catch {
    return {
      raw,
      connections: null,
      failure: {
        code: 'migration_incomplete',
        message: '旧版连接数据无法解析，Astesia 未删除任何数据。',
        remediation: '请先备份 localStorage 中的 astesia_connections，再修复其 JSON 格式。',
      },
    };
  }
};

const repositoryFailure = (error: unknown): MigrationFailure => {
  const value = error as Partial<ConnectionRepositoryError>;
  const details = value?.details;
  const missingIds = Array.isArray(details?.missing_ids) ? details.missing_ids : [];
  const conflictingIds = Array.isArray(details?.conflicting_ids)
    ? details.conflicting_ids
    : [];
  const affectedIds = [...missingIds, ...conflictingIds]
    .filter((id): id is string => typeof id === 'string');
  const affectedSummary = affectedIds.length > 0
    ? `受影响连接：${affectedIds.join('、')}`
    : undefined;
  return {
    code: value?.code,
    message: [value?.message || String(error), affectedSummary]
      .filter(Boolean)
      .join(' '),
    remediation: value?.remediation,
  };
};

const removeLegacySnapshot = (expectedRaw: string | null) => {
  if (localStorage.getItem('astesia_connections') !== expectedRaw) {
    throw {
      code: 'migration_incomplete',
      message: '迁移期间旧版连接数据发生变化，Astesia 未删除该数据。',
      remediation: '请重新检查连接并再次执行迁移。',
    } satisfies MigrationFailure;
  }
  localStorage.removeItem('astesia_connections');
};

function App() {
  const { setConnections, applyMcpConnectionsSnapshot } = useConnectionStore();
  const initTheme = useThemeStore((s) => s.initTheme);
  const checkForUpdates = useUpdateStore((s) => s.checkForUpdates);
  const [migrationPhase, setMigrationPhase] = useState<MigrationPhase>('checking');
  const [legacyConnections, setLegacyConnections] = useState<ConnectionConfig[] | null>([]);
  const [migrationFailure, setMigrationFailure] = useState<MigrationFailure | null>(null);
  const sharedRevision = useRef(-1);
  const hydrationRequest = useRef(0);

  useEffect(() => {
    const cleanupTheme = initTheme();
    const savedLang = localStorage.getItem('astesia_language');
    if (savedLang) i18n.changeLanguage(savedLang);
    return () => {
      cleanupTheme();
    };
  }, [initTheme]);

  const hydrateSharedConnections = useCallback(async () => {
    const request = ++hydrationRequest.current;
    const snapshot = await invoke<ConnectionProfilesSnapshot>(
      'connection_profiles_snapshot'
    );
    if (request !== hydrationRequest.current) return;
    setConnections(snapshot.profiles.map(sharedProfileToConfig));
    sharedRevision.current = snapshot.revision;
  }, [setConnections]);

  useEffect(() => {
    if (!isTauri()) {
      const { connections } = readLegacyConnections();
      if (connections) {
        setConnections(connections);
        localStorage.setItem(
          'astesia_connections',
          JSON.stringify(connections.map((connection) => ({
            ...connection,
            password: '',
          })))
        );
      }
      const unsubscribe = useConnectionStore.subscribe((state) => {
        localStorage.setItem(
          'astesia_connections',
          JSON.stringify(
            state.connections.map((connection) => ({ ...connection, password: '' }))
          )
        );
      });
      setMigrationPhase('ready');
      return unsubscribe;
    }

    let disposed = false;
    const initializeConnections = async () => {
      const legacy = readLegacyConnections();
      if (disposed) return;
      setLegacyConnections(legacy.connections);
      setMigrationFailure(legacy.failure);
      if (legacy.failure || (legacy.connections?.length ?? 0) > 0) {
        setMigrationPhase('required');
        return;
      }

      try {
        // Hydrating shared profile metadata must not unlock the credential
        // vault. Password access is deferred until the user connects, tests,
        // saves, deletes, or explicitly starts an MCP credential migration.
        removeLegacySnapshot(legacy.raw);
        await hydrateSharedConnections();
        if (!disposed) setMigrationPhase('ready');
      } catch (error) {
        if (!disposed) {
          setMigrationFailure(repositoryFailure(error));
          setMigrationPhase('required');
        }
      }
    };
    void initializeConnections();
    return () => {
      disposed = true;
      hydrationRequest.current += 1;
    };
  }, [hydrateSharedConnections, setConnections]);

  const migrateLegacyConnections = useCallback(async () => {
    const latestLegacy = readLegacyConnections();
    setLegacyConnections(latestLegacy.connections);
    if (!latestLegacy.connections || latestLegacy.failure) {
      setMigrationFailure(latestLegacy.failure);
      setMigrationPhase('required');
      return;
    }
    setMigrationPhase('migrating');
    setMigrationFailure(null);
    try {
      const result = await invoke<LegacyMigrationResult>('migrate_legacy_connections', {
        connections: latestLegacy.connections,
      });
      if (result.imported + result.skipped !== latestLegacy.connections.length) {
        throw new Error('迁移结果未覆盖全部旧连接，旧数据未被删除。');
      }
      removeLegacySnapshot(latestLegacy.raw);
      setLegacyConnections([]);
      await hydrateSharedConnections();
      setMigrationPhase('ready');
    } catch (error) {
      setMigrationFailure(repositoryFailure(error));
      setMigrationPhase('required');
    }
  }, [hydrateSharedConnections]);

  useEffect(() => {
    if (!isTauri() || migrationPhase !== 'ready') return;
    let disposed = false;
    let checking = false;
    const refreshIfChanged = async () => {
      if (checking) return;
      checking = true;
      try {
        const revision = await invoke<number>('shared_connections_revision');
        if (!disposed && revision !== sharedRevision.current) {
          await hydrateSharedConnections();
        }
      } catch (error) {
        console.error('Failed to refresh shared connections:', error);
      } finally {
        checking = false;
      }
    };
    const timer = window.setInterval(() => void refreshIfChanged(), 1500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, [hydrateSharedConnections, migrationPhase]);

  useEffect(() => {
    if (!isTauri() || migrationPhase !== 'ready') return;

    let disposed = false;
    let unlisten: UnlistenFn | undefined;

    const initializeMcpConnectionSync = async () => {
      try {
        unlisten = await listen<McpConnectionsSnapshot>(
          'mcp-connections-changed',
          (event) => {
            if (!disposed) applyMcpConnectionsSnapshot(event.payload);
          }
        );
        if (disposed) {
          unlisten();
          return;
        }

        const snapshot = await invoke<McpConnectionsSnapshot>('mcp_synced_connections');
        if (!disposed) applyMcpConnectionsSnapshot(snapshot);
      } catch (error) {
        console.error('Failed to initialize MCP connection sync:', error);
      }
    };

    void initializeMcpConnectionSync();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [applyMcpConnectionsSnapshot, migrationPhase]);

  // Check for updates after startup
  useEffect(() => {
    const timer = setTimeout(() => {
      checkForUpdates(true);
    }, 3000);
    return () => clearTimeout(timer);
  }, [checkForUpdates]);

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

  if (migrationPhase !== 'ready') {
    return (
      <>
        {migrationPhase === 'checking' ? (
          <div className="fixed inset-0 bg-background" />
        ) : (
          <ConnectionMigrationGate
            connectionCount={legacyConnections?.length ?? 0}
            canMigrate={legacyConnections !== null}
            migrating={migrationPhase === 'migrating'}
            failure={migrationFailure}
            onMigrate={() => void migrateLegacyConnections()}
          />
        )}
        <ToastContainer />
      </>
    );
  }

  return (
    <>
      <AppLayout />
      <ToastContainer />
      <CreateResourceDialog />
      <ConfirmDialog />
      <UpdateDialog />
    </>
  );
}

export default App;
