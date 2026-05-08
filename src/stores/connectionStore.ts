import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { ConnectionConfig, ConnectionResult, FunctionInfo, ProcedureInfo, TableInfo, TriggerInfo, UserInfo, ViewInfo } from '@/types/database';
import { notify } from '@/stores/notificationStore';

interface TreeNode {
  connectionId: string;
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

  addConnection: (config: ConnectionConfig) => void;
  removeConnection: (id: string) => void;
  updateConnection: (config: ConnectionConfig) => void;
  setConnections: (connections: ConnectionConfig[]) => void;

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

/// Canonical loading-key builders, kept here so producers and consumers
/// (Sidebar, tree components) agree on the exact string shape.
export const loadingKey = {
  databases: (connId: string) => `databases:${connId}`,
  schemas: (connId: string, db: string) => `schemas:${connId}:${db}`,
  tables: (connId: string, db: string) => `tables:${connId}:${db}`,
  views: (connId: string, db: string) => `views:${connId}:${db}`,
  functions: (connId: string, db: string) => `functions:${connId}:${db}`,
  procedures: (connId: string, db: string) => `procedures:${connId}:${db}`,
  triggers: (connId: string, db: string) => `triggers:${connId}:${db}`,
  users: (connId: string) => `users:${connId}`,
};

export const useConnectionStore = create<ConnectionStore>((set, get) => ({
  connections: [],
  treeData: {},
  connectingIds: new Set<string>(),
  loadingKeys: new Set<string>(),
  activeConnectionId: null,
  activeDatabase: null,

  addConnection: (config) =>
    set((state) => ({
      connections: [...state.connections, config],
    })),

  removeConnection: (id) =>
    set((state) => ({
      connections: state.connections.filter((c) => c.id !== id),
      treeData: Object.fromEntries(
        Object.entries(state.treeData).filter(([k]) => k !== id)
      ),
    })),

  updateConnection: (config) =>
    set((state) => ({
      connections: state.connections.map((c) =>
        c.id === config.id ? config : c
      ),
    })),

  setConnections: (connections) => set({ connections }),

  connectDatabase: async (id) => {
    const config = get().connections.find((c) => c.id === id);
    if (!config) return { success: false, message: '连接配置不存在' };
    if (get().connectingIds.has(id)) {
      return { success: false, message: '正在连接中' };
    }

    set((state) => ({ connectingIds: addToSet(state.connectingIds, id) }));
    try {
      const result = await invoke<ConnectionResult>('connect_database', { config });
      if (result.success) {
        set((state) => ({
          treeData: {
            ...state.treeData,
            [id]: {
              connectionId: id,
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
            },
          },
          activeConnectionId: id,
        }));
        await get().loadDatabases(id);
      } else {
        notify.error('连接失败', result.message);
      }
      return result;
    } finally {
      set((state) => ({ connectingIds: removeFromSet(state.connectingIds, id) }));
    }
  },

  disconnectDatabase: async (id) => {
    await invoke('disconnect_database', { connectionId: id });
    set((state) => {
      const newTreeData = { ...state.treeData };
      delete newTreeData[id];
      return {
        treeData: newTreeData,
        activeConnectionId:
          state.activeConnectionId === id ? null : state.activeConnectionId,
      };
    });
  },

  testConnection: async (config) => {
    return await invoke<ConnectionResult>('test_connection', { config });
  },

  loadDatabases: async (connectionId) => {
    const key = loadingKey.databases(connectionId);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const databases = await invoke<string[]>('get_databases', { connectionId });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            databases,
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load databases:', e);
      notify.error('加载数据库失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadSchemas: async (connectionId, database) => {
    const key = loadingKey.schemas(connectionId, database);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const schemas = await invoke<string[]>('get_schemas', { connectionId, database });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            schemas: {
              ...state.treeData[connectionId]?.schemas,
              [database]: schemas,
            },
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load schemas:', e);
      notify.error('加载Schema失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadTables: async (connectionId, database) => {
    const key = loadingKey.tables(connectionId, database);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const tables = await invoke<TableInfo[]>('get_tables', { connectionId, database });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            tables: {
              ...state.treeData[connectionId]?.tables,
              [database]: tables,
            },
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load tables:', e);
      notify.error('加载表失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadViews: async (connectionId, database) => {
    const key = loadingKey.views(connectionId, database);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const views = await invoke<ViewInfo[]>('get_views', { connectionId, database });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            views: {
              ...state.treeData[connectionId]?.views,
              [database]: views,
            },
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load views:', e);
      notify.error('加载视图失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadFunctions: async (connectionId, database) => {
    const key = loadingKey.functions(connectionId, database);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const functions = await invoke<FunctionInfo[]>('get_functions', { connectionId, database });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            functions: {
              ...state.treeData[connectionId]?.functions,
              [database]: functions,
            },
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load functions:', e);
      notify.error('加载函数失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadProcedures: async (connectionId, database) => {
    const key = loadingKey.procedures(connectionId, database);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const procedures = await invoke<ProcedureInfo[]>('get_procedures', { connectionId, database });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            procedures: {
              ...state.treeData[connectionId]?.procedures,
              [database]: procedures,
            },
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load procedures:', e);
      notify.error('加载存储过程失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadTriggers: async (connectionId, database) => {
    const key = loadingKey.triggers(connectionId, database);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const triggers = await invoke<TriggerInfo[]>('get_triggers', { connectionId, database });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            triggers: {
              ...state.treeData[connectionId]?.triggers,
              [database]: triggers,
            },
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load triggers:', e);
      notify.error('加载触发器失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
  },

  loadUsers: async (connectionId) => {
    const key = loadingKey.users(connectionId);
    set((state) => ({ loadingKeys: addToSet(state.loadingKeys, key) }));
    try {
      const users = await invoke<UserInfo[]>('get_users', { connectionId });
      set((state) => ({
        treeData: {
          ...state.treeData,
          [connectionId]: {
            ...state.treeData[connectionId],
            users,
          },
        },
      }));
    } catch (e) {
      console.error('Failed to load users:', e);
      notify.error('加载用户失败', typeof e === 'string' ? e : String(e));
    } finally {
      set((state) => ({ loadingKeys: removeFromSet(state.loadingKeys, key) }));
    }
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
}));
