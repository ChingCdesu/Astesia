import { create } from 'zustand';
import { invoke, isTauri } from '@tauri-apps/api/core';
import {
  ConnectionConfig,
  ConnectionRepositoryError,
  ConnectionResult,
  DisconnectConnectionResult,
  FunctionInfo,
  McpConnectionUsage,
  McpConnectionsSnapshot,
  ProcedureInfo,
  SharedConnectionProfile,
  TableInfo,
  TriggerInfo,
  UserInfo,
  ViewInfo,
} from '@/types/database';
import { notify } from '@/stores/notificationStore';

interface DeleteConnectionProfileResult {
  deleted: boolean;
  revision: number;
  credential_cleanup_pending: boolean;
}

interface TreeNode {
  connectionId: string;
  epoch: number;
  databases: string[];
  schemas: Record<string, string[]>;
  tables: Record<string, TableInfo[]>;
  views: Record<string, ViewInfo[]>;
  functions: Record<string, FunctionInfo[]>;
  procedures: Record<string, ProcedureInfo[]>;
  triggers: Record<string, TriggerInfo[]>;
  users: UserInfo[];
  expanded: Set<string>;
  connected: boolean;
}

interface ConnectionStore {
  connections: ConnectionConfig[];
  mcpRevision: number;
  mcpUsageByConnectionId: Record<string, McpConnectionUsage>;
  treeData: Record<string, TreeNode>;
  /// Connection IDs currently mid-connect. Sidebar reads this to render a
  /// spinner on the connection node while the user waits for the network
  /// handshake to finish.
  connectingIds: Set<string>;
  /// Granular loading keys for sub-resources, e.g. `tables:${connId}:${db}`.
  /// Components key off this set to render per-node spinners during expand /
  /// refresh operations.
  loadingKeys: Set<string>;
  activeConnectionId: string | null;
  activeDatabase: string | null;

  addConnection: (config: ConnectionConfig) => Promise<void>;
  removeConnection: (id: string) => Promise<void>;
  updateConnection: (config: ConnectionConfig) => Promise<void>;
  setConnections: (connections: ConnectionConfig[]) => void;
  applyMcpConnectionsSnapshot: (snapshot: McpConnectionsSnapshot) => void;

  connectDatabase: (id: string) => Promise<ConnectionResult>;
  disconnectDatabase: (id: string) => Promise<void>;
  testConnection: (config: ConnectionConfig) => Promise<ConnectionResult>;

  loadDatabases: (connectionId: string) => Promise<void>;
  loadSchemas: (connectionId: string, database: string) => Promise<void>;
  loadTables: (connectionId: string, database: string) => Promise<void>;
  loadViews: (connectionId: string, database: string) => Promise<void>;
  loadFunctions: (connectionId: string, database: string) => Promise<void>;
  loadProcedures: (connectionId: string, database: string) => Promise<void>;
  loadTriggers: (connectionId: string, database: string) => Promise<void>;
  loadUsers: (connectionId: string) => Promise<void>;

  setActiveConnection: (id: string | null) => void;
  setActiveDatabase: (db: string | null) => void;
  toggleExpand: (connectionId: string, key: string) => void;
}

const addToSet = <T>(set: Set<T>, value: T): Set<T> => {
  const next = new Set(set);
  next.add(value);
  return next;
};

const removeFromSet = <T>(set: Set<T>, value: T): Set<T> => {
  if (!set.has(value)) return set;
  const next = new Set(set);
  next.delete(value);
  return next;
};

const repositoryErrorMessage = (error: unknown): string => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  if (error && typeof error === 'object') {
    const repositoryError = error as Partial<ConnectionRepositoryError>;
    const message = typeof repositoryError.message === 'string'
      ? repositoryError.message
      : undefined;
    const remediation = typeof repositoryError.remediation === 'string'
      ? repositoryError.remediation
      : undefined;
    const code = typeof repositoryError.code === 'string'
      ? `错误码：${repositoryError.code}`
      : undefined;
    const parts = [message, remediation, code].filter(
      (part): part is string => Boolean(part)
    );
    if (parts.length > 0) return parts.join(' ');
  }
  return String(error);
};

export const sharedProfileToConfig = (
  profile: SharedConnectionProfile
): ConnectionConfig => ({
  ...profile,
  password: '',
  source: 'shared',
});

const backendConnectionConfig = (
  config: ConnectionConfig
): ConnectionConfig => ({
  id: config.id,
  name: config.name,
  db_type: config.db_type,
  host: config.host,
  port: config.port,
  username: config.username,
  password: config.password,
  database: config.database,
  color: config.color,
});

const isMcpProfileLocked = (connection: ConnectionConfig): boolean =>
  connection.mcp_in_use === true || connection.disconnecting === true;

const withMcpUsage = (
  connection: ConnectionConfig,
  usage?: McpConnectionUsage
): ConnectionConfig => {
  const currentUsage =
    Number.isSafeInteger(connection.revision)
    && usage?.profile_revision === connection.revision
      ? usage
      : undefined;

  return {
    ...connection,
    mcp_in_use: currentUsage?.mcp_in_use ?? false,
    mcp_connected: currentUsage?.mcp_connected ?? false,
    mcp_session_count: currentUsage?.mcp_session_count ?? 0,
    disconnecting: currentUsage?.disconnecting ?? false,
    last_error: currentUsage?.last_error ?? null,
  };
};

const isValidMcpConnectionUsage = (
  value: unknown
): value is McpConnectionUsage => {
  if (!value || typeof value !== 'object') return false;
  const usage = value as Partial<McpConnectionUsage>;
  return typeof usage.id === 'string'
    && usage.id.length > 0
    && Number.isSafeInteger(usage.profile_revision)
    && typeof usage.mcp_in_use === 'boolean'
    && typeof usage.mcp_connected === 'boolean'
    && Number.isSafeInteger(usage.mcp_session_count)
    && (usage.mcp_session_count ?? -1) >= 0
    && typeof usage.disconnecting === 'boolean'
    && (
      usage.last_error === undefined
      || usage.last_error === null
      || typeof usage.last_error === 'string'
    );
};

let nextTreeEpoch = 0;
let nextTreeRequest = 0;
const latestTreeRequests = new Map<string, number>();

const emptyConnectedTreeNode = (connectionId: string): TreeNode => ({
  connectionId,
  epoch: ++nextTreeEpoch,
  databases: [],
  schemas: {},
  tables: {},
  views: {},
  functions: {},
  procedures: {},
  triggers: {},
  users: [],
  expanded: new Set(),
  connected: true,
});

const isTreeLoadCurrent = (
  state: Pick<ConnectionStore, 'connections' | 'treeData'>,
  connectionId: string,
  epoch: number
): boolean => {
  const node = state.treeData[connectionId];
  const connection = state.connections.find((item) => item.id === connectionId);
  return node?.connected === true
    && node.epoch === epoch
    && connection !== undefined;
};

const withoutConnectionLoadingKeys = (
  loadingKeys: Set<string>,
  connectionId: string
): Set<string> => {
  const encodedConnectionId = encodeURIComponent(connectionId);
  const exactKeys = new Set([
    loadingKey.databases(connectionId),
    loadingKey.users(connectionId),
  ]);
  const prefixes = [
    `schemas:${encodedConnectionId}:`,
    `tables:${encodedConnectionId}:`,
    `views:${encodedConnectionId}:`,
    `functions:${encodedConnectionId}:`,
    `procedures:${encodedConnectionId}:`,
    `triggers:${encodedConnectionId}:`,
  ];
  const next = new Set(
    [...loadingKeys].filter(
      (key) => !exactKeys.has(key) && !prefixes.some((prefix) => key.startsWith(prefix))
    )
  );
  return next.size === loadingKeys.size ? loadingKeys : next;
};

/// Canonical loading-key builders, kept here so producers and consumers
/// (Sidebar, tree components) agree on the exact string shape.
export const loadingKey = {
  databases: (connId: string) => `databases:${encodeURIComponent(connId)}`,
  schemas: (connId: string, db: string) =>
    `schemas:${encodeURIComponent(connId)}:${encodeURIComponent(db)}`,
  tables: (connId: string, db: string) =>
    `tables:${encodeURIComponent(connId)}:${encodeURIComponent(db)}`,
  views: (connId: string, db: string) =>
    `views:${encodeURIComponent(connId)}:${encodeURIComponent(db)}`,
  functions: (connId: string, db: string) =>
    `functions:${encodeURIComponent(connId)}:${encodeURIComponent(db)}`,
  procedures: (connId: string, db: string) =>
    `procedures:${encodeURIComponent(connId)}:${encodeURIComponent(db)}`,
  triggers: (connId: string, db: string) =>
    `triggers:${encodeURIComponent(connId)}:${encodeURIComponent(db)}`,
  users: (connId: string) => `users:${encodeURIComponent(connId)}`,
};

export const useConnectionStore = create<ConnectionStore>((set, get) => {
  const loadTreeResource = async <T>(
    connectionId: string,
    key: string,
    load: () => Promise<T>,
    errorLabel: string,
    apply: (node: TreeNode, value: T) => TreeNode
  ): Promise<void> => {
    const epoch = get().treeData[connectionId]?.epoch;
    if (epoch === undefined || !isTreeLoadCurrent(get(), connectionId, epoch)) {
      return;
    }
    const request = ++nextTreeRequest;
    latestTreeRequests.set(key, request);

    set((state) => (
      isTreeLoadCurrent(state, connectionId, epoch)
        ? { loadingKeys: addToSet(state.loadingKeys, key) }
        : state
    ));
    try {
      const value = await load();
      set((state) => {
        if (
          latestTreeRequests.get(key) !== request
          || !isTreeLoadCurrent(state, connectionId, epoch)
        ) {
          return state;
        }
        const node = state.treeData[connectionId];
        return {
          treeData: {
            ...state.treeData,
            [connectionId]: apply(node, value),
          },
        };
      });
    } catch (error) {
      if (
        latestTreeRequests.get(key) === request
        && isTreeLoadCurrent(get(), connectionId, epoch)
      ) {
        console.error(`Failed to load ${errorLabel}:`, error);
        notify.error(
          `加载${errorLabel}失败`,
          typeof error === 'string' ? error : String(error)
        );
      }
    } finally {
      if (latestTreeRequests.get(key) === request) {
        latestTreeRequests.delete(key);
        set((state) => (
          state.treeData[connectionId]?.epoch === epoch
            ? { loadingKeys: removeFromSet(state.loadingKeys, key) }
            : state
        ));
      }
    }
  };

  return {
  connections: [],
  mcpRevision: -1,
  mcpUsageByConnectionId: {},
  treeData: {},
  connectingIds: new Set<string>(),
  loadingKeys: new Set<string>(),
  activeConnectionId: null,
  activeDatabase: null,

  addConnection: async (config) => {
    if (!isTauri()) {
      set((state) => ({
        connections: [
          ...state.connections,
          withMcpUsage(config, state.mcpUsageByConnectionId[config.id]),
        ],
      }));
      return;
    }
    const profile = await invoke<SharedConnectionProfile>('save_connection_profile', {
      request: {
        config: backendConnectionConfig(config),
        expected_revision: null,
        mcp_enabled: true,
      },
    });
    set((state) => ({
      connections: [
        ...state.connections,
        withMcpUsage(
          sharedProfileToConfig(profile),
          state.mcpUsageByConnectionId[profile.id]
        ),
      ],
    }));
  },

  removeConnection: async (id) => {
    const existing = get().connections.find((connection) => connection.id === id);
    if (!existing) {
      throw new Error('连接配置不存在，请刷新连接列表后重试');
    }
    if (isMcpProfileLocked(existing)) {
      throw new Error('连接正被 MCP 使用或断开中，请先断开后再删除');
    }
    if (isTauri()) {
      if (!Number.isSafeInteger(existing.revision)) {
        throw new Error('连接缺少有效 revision，请刷新连接列表后重试');
      }
      const result = await invoke<DeleteConnectionProfileResult>('delete_connection_profile', {
        connectionId: id,
        expectedRevision: existing.revision,
      });
      if (result.credential_cleanup_pending) {
        notify.warning(
          '连接已删除，凭据清理待完成',
          '系统凭据库暂时无法删除该连接的旧凭据；Astesia 已记录清理任务。'
        );
      }
    }
    set((state) => {
      const nextTreeData = { ...state.treeData };
      delete nextTreeData[id];
      const resetActiveConnection = state.activeConnectionId === id;
      return {
        connections: state.connections.filter((c) => c.id !== id),
        treeData: nextTreeData,
        loadingKeys: withoutConnectionLoadingKeys(state.loadingKeys, id),
        connectingIds: removeFromSet(state.connectingIds, id),
        activeConnectionId: resetActiveConnection ? null : state.activeConnectionId,
        activeDatabase: resetActiveConnection ? null : state.activeDatabase,
      };
    });
  },

  updateConnection: async (config) => {
    const existing = get().connections.find(
      (connection) => connection.id === config.id
    );
    if (!existing) {
      throw new Error('连接配置不存在，请刷新连接列表后重试');
    }
    if (isMcpProfileLocked(existing)) {
      throw new Error('连接正被 MCP 使用或断开中，请先断开后再编辑');
    }
    if (!isTauri()) {
      set((state) => ({
        connections: state.connections.map((connection) =>
          connection.id === config.id
            ? withMcpUsage(config, state.mcpUsageByConnectionId[config.id])
            : connection
        ),
      }));
      return;
    }
    if (!Number.isSafeInteger(config.revision)) {
      throw new Error('连接缺少有效 revision，请刷新连接列表后重试');
    }
    const profile = await invoke<SharedConnectionProfile>('save_connection_profile', {
      request: {
        config: backendConnectionConfig(config),
        expected_revision: config.revision,
        mcp_enabled: config.mcp_enabled ?? true,
      },
    });
    set((state) => {
      const nextTreeData = { ...state.treeData };
      delete nextTreeData[config.id];
      const resetActiveConnection = state.activeConnectionId === config.id;
      return {
        connections: state.connections.map((connection) =>
          connection.id === config.id
            ? withMcpUsage(
              sharedProfileToConfig(profile),
              state.mcpUsageByConnectionId[profile.id]
            )
            : connection
        ),
        treeData: nextTreeData,
        loadingKeys: withoutConnectionLoadingKeys(state.loadingKeys, config.id),
        connectingIds: removeFromSet(state.connectingIds, config.id),
        activeConnectionId: resetActiveConnection ? null : state.activeConnectionId,
        activeDatabase: resetActiveConnection ? null : state.activeDatabase,
      };
    });
  },

  setConnections: (connections) =>
    set((state) => {
      const nextConnections = connections.map((connection) =>
        withMcpUsage(
          connection,
          state.mcpUsageByConnectionId[connection.id]
        )
      );
      const nextConnectionsById = new Map(
        nextConnections.map((connection) => [connection.id, connection])
      );
      const invalidatedIds = new Set(
        state.connections
          .filter((connection) => {
            const next = nextConnectionsById.get(connection.id);
            return !next || next.revision !== connection.revision;
          })
          .map((connection) => connection.id)
      );

      const nextTreeData = { ...state.treeData };
      let nextLoadingKeys = state.loadingKeys;
      let nextConnectingIds = state.connectingIds;
      for (const connectionId of invalidatedIds) {
        delete nextTreeData[connectionId];
        nextLoadingKeys = withoutConnectionLoadingKeys(nextLoadingKeys, connectionId);
        nextConnectingIds = removeFromSet(nextConnectingIds, connectionId);
      }
      const resetActiveConnection = state.activeConnectionId !== null
        && invalidatedIds.has(state.activeConnectionId);

      return {
        connections: nextConnections,
        treeData: nextTreeData,
        loadingKeys: nextLoadingKeys,
        connectingIds: nextConnectingIds,
        activeConnectionId: resetActiveConnection ? null : state.activeConnectionId,
        activeDatabase: resetActiveConnection ? null : state.activeDatabase,
      };
    }),

  applyMcpConnectionsSnapshot: (snapshot) => {
    if (
      !Number.isSafeInteger(snapshot.revision)
      || snapshot.revision < 0
      || !Array.isArray(snapshot.connections)
      || !snapshot.connections.every(isValidMcpConnectionUsage)
    ) {
      console.error('Invalid MCP connections snapshot:', snapshot);
      return;
    }

    set((state) => {
      if (snapshot.revision <= state.mcpRevision) return state;

      const mcpUsageByConnectionId: Record<string, McpConnectionUsage> =
        Object.fromEntries(
          snapshot.connections.map((connection) => [connection.id, connection])
        );

      return {
        mcpRevision: snapshot.revision,
        mcpUsageByConnectionId,
        connections: state.connections.map((connection) =>
          withMcpUsage(
            connection,
            mcpUsageByConnectionId[connection.id]
          )
        ),
      };
    });
  },

  connectDatabase: async (id) => {
    const config = get().connections.find((c) => c.id === id);
    if (!config) return { success: false, message: '连接配置不存在' };
    if (get().connectingIds.has(id)) {
      return { success: false, message: '正在连接中' };
    }

    set((state) => ({ connectingIds: addToSet(state.connectingIds, id) }));
    try {
      const result = await invoke<ConnectionResult>('connect_database', {
        connectionId: config.id,
      });
      if (result.success) {
        set((state) => ({
          treeData: {
            ...state.treeData,
            [id]: emptyConnectedTreeNode(id),
          },
          activeConnectionId: id,
        }));
        await get().loadDatabases(id);
      } else {
        notify.error('连接失败', result.message);
      }
      return result;
    } catch (error) {
      const result = {
        success: false,
        message: repositoryErrorMessage(error),
      };
      notify.error('连接失败', result.message);
      return result;
    } finally {
      set((state) => ({ connectingIds: removeFromSet(state.connectingIds, id) }));
    }
  },

  disconnectDatabase: async (id) => {
    try {
      const result = await invoke<DisconnectConnectionResult>(
        'disconnect_database',
        { connectionId: id }
      );
      if (result.partial) {
        notify.warning('连接未完全断开', result.message);
      } else if (!result.success) {
        notify.info('连接状态', result.message);
      }
    } catch (error) {
      notify.error('断开连接失败', repositoryErrorMessage(error));
    } finally {
      set((state) => {
        const newTreeData = { ...state.treeData };
        delete newTreeData[id];
        return {
          treeData: newTreeData,
          loadingKeys: withoutConnectionLoadingKeys(state.loadingKeys, id),
          activeConnectionId:
            state.activeConnectionId === id ? null : state.activeConnectionId,
          activeDatabase:
            state.activeConnectionId === id ? null : state.activeDatabase,
        };
      });
    }
  },

  testConnection: async (config) => {
    return await invoke<ConnectionResult>('test_connection', { config });
  },

  loadDatabases: async (connectionId) => {
    await loadTreeResource(
      connectionId,
      loadingKey.databases(connectionId),
      () => invoke<string[]>('get_databases', { connectionId }),
      '数据库',
      (node, databases) => ({ ...node, databases })
    );
  },

  loadSchemas: async (connectionId, database) => {
    await loadTreeResource(
      connectionId,
      loadingKey.schemas(connectionId, database),
      () => invoke<string[]>('get_schemas', { connectionId, database }),
      'Schema',
      (node, schemas) => ({
        ...node,
        schemas: { ...node.schemas, [database]: schemas },
      })
    );
  },

  loadTables: async (connectionId, database) => {
    await loadTreeResource(
      connectionId,
      loadingKey.tables(connectionId, database),
      () => invoke<TableInfo[]>('get_tables', { connectionId, database }),
      '表',
      (node, tables) => ({
        ...node,
        tables: { ...node.tables, [database]: tables },
      })
    );
  },

  loadViews: async (connectionId, database) => {
    await loadTreeResource(
      connectionId,
      loadingKey.views(connectionId, database),
      () => invoke<ViewInfo[]>('get_views', { connectionId, database }),
      '视图',
      (node, views) => ({
        ...node,
        views: { ...node.views, [database]: views },
      })
    );
  },

  loadFunctions: async (connectionId, database) => {
    await loadTreeResource(
      connectionId,
      loadingKey.functions(connectionId, database),
      () => invoke<FunctionInfo[]>('get_functions', { connectionId, database }),
      '函数',
      (node, functions) => ({
        ...node,
        functions: { ...node.functions, [database]: functions },
      })
    );
  },

  loadProcedures: async (connectionId, database) => {
    await loadTreeResource(
      connectionId,
      loadingKey.procedures(connectionId, database),
      () => invoke<ProcedureInfo[]>('get_procedures', { connectionId, database }),
      '存储过程',
      (node, procedures) => ({
        ...node,
        procedures: { ...node.procedures, [database]: procedures },
      })
    );
  },

  loadTriggers: async (connectionId, database) => {
    await loadTreeResource(
      connectionId,
      loadingKey.triggers(connectionId, database),
      () => invoke<TriggerInfo[]>('get_triggers', { connectionId, database }),
      '触发器',
      (node, triggers) => ({
        ...node,
        triggers: { ...node.triggers, [database]: triggers },
      })
    );
  },

  loadUsers: async (connectionId) => {
    await loadTreeResource(
      connectionId,
      loadingKey.users(connectionId),
      () => invoke<UserInfo[]>('get_users', { connectionId }),
      '用户',
      (node, users) => ({ ...node, users })
    );
  },

  setActiveConnection: (id) => set({ activeConnectionId: id }),
  setActiveDatabase: (db) => set({ activeDatabase: db }),
  toggleExpand: (connectionId, key) =>
    set((state) => {
      const node = state.treeData[connectionId];
      if (!node) return state;
      const expanded = new Set(node.expanded);
      if (expanded.has(key)) {
        expanded.delete(key);
      } else {
        expanded.add(key);
      }
      return {
        treeData: {
          ...state.treeData,
          [connectionId]: { ...node, expanded },
        },
      };
    }),
  };
});
