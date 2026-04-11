import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { ConnectionConfig, ConnectionResult, FunctionInfo, ProcedureInfo, TableInfo, TriggerInfo, UserInfo, ViewInfo } from '@/types/database';

interface TreeNode {
  connectionId: string;
  databases: string[];
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

export const useConnectionStore = create<ConnectionStore>((set, get) => ({
  connections: [],
  treeData: {},
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

    const result = await invoke<ConnectionResult>('connect_database', { config });
    if (result.success) {
      set((state) => ({
        treeData: {
          ...state.treeData,
          [id]: {
            connectionId: id,
            databases: [],
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
    }
    return result;
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
    try {
      const databases = await invoke<string[]>('get_databases', {
        connectionId,
      });
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
    }
  },

  loadTables: async (connectionId, database) => {
    try {
      const tables = await invoke<TableInfo[]>('get_tables', {
        connectionId,
        database,
      });
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
    }
  },

  loadViews: async (connectionId, database) => {
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
    }
  },

  loadFunctions: async (connectionId, database) => {
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
    }
  },

  loadProcedures: async (connectionId, database) => {
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
    }
  },

  loadTriggers: async (connectionId, database) => {
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
    }
  },

  loadUsers: async (connectionId) => {
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
