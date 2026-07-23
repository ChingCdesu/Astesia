import { invoke, isTauri } from '@tauri-apps/api/core';
import { create } from 'zustand';
import i18n from '@/i18n';
import { notify } from '@/stores/notificationStore';

export type McpServiceState = 'stopped' | 'starting' | 'running' | 'stopping' | 'error';
export type McpServiceOperation = 'start' | 'stop' | 'restart';

export interface McpServiceStatus {
  state: McpServiceState;
  available: boolean;
  pid: number | null;
  endpoint: string | null;
  transport: 'streamable_http';
  binary_path: string | null;
  version: string | null;
  started_at: string | null;
  last_error: string | null;
}

interface McpHelperStore {
  status: McpServiceStatus;
  port: number;
  authToken: string;
  operation: McpServiceOperation | null;

  setPort: (port: number) => void;
  rotateAuthToken: () => string;
  refreshStatus: () => Promise<void>;
  startService: () => Promise<void>;
  stopService: () => Promise<void>;
  restartService: () => Promise<void>;
}

const PORT_KEY = 'astesia_mcp_port';
const AUTH_TOKEN_KEY = 'astesia_mcp_auth_token';
const DEFAULT_PORT = 43677;
const MIN_PORT = 1024;
const MAX_PORT = 65535;

const INITIAL_STATUS: McpServiceStatus = {
  state: 'stopped',
  available: false,
  pid: null,
  endpoint: null,
  transport: 'streamable_http',
  binary_path: null,
  version: null,
  started_at: null,
  last_error: null,
};

let statusRequestSequence = 0;

function readStoredPort(): number {
  const saved = Number(localStorage.getItem(PORT_KEY));
  return Number.isInteger(saved) && saved >= MIN_PORT && saved <= MAX_PORT
    ? saved
    : DEFAULT_PORT;
}

function createAuthToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
}

function readOrCreateAuthToken(): string {
  const saved = localStorage.getItem(AUTH_TOKEN_KEY);
  if (saved && /^[a-f0-9]{64}$/i.test(saved)) {
    return saved;
  }

  const token = createAuthToken();
  localStorage.setItem(AUTH_TOKEN_KEY, token);
  return token;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isMcpServiceStatus(value: unknown): value is McpServiceStatus {
  if (!value || typeof value !== 'object') return false;
  const state = (value as { state?: unknown }).state;
  return (
    state === 'stopped'
    || state === 'starting'
    || state === 'running'
    || state === 'stopping'
    || state === 'error'
  );
}

function requireServiceState(
  value: unknown,
  expectedState: McpServiceState,
): McpServiceStatus {
  if (!isMcpServiceStatus(value)) {
    throw new Error(i18n.t('mcpHelper.invalidResponse'));
  }
  if (value.state !== expectedState) {
    throw new Error(value.last_error ?? i18n.t('mcpHelper.operationIncomplete'));
  }
  return value;
}

export const useMcpHelperStore = create<McpHelperStore>((set, get) => ({
  status: INITIAL_STATUS,
  port: readStoredPort(),
  authToken: readOrCreateAuthToken(),
  operation: null,

  setPort: (port) => {
    if (!Number.isInteger(port) || port < MIN_PORT || port > MAX_PORT) return;
    localStorage.setItem(PORT_KEY, String(port));
    set({ port });
  },

  rotateAuthToken: () => {
    const authToken = createAuthToken();
    localStorage.setItem(AUTH_TOKEN_KEY, authToken);
    set({ authToken });
    return authToken;
  },

  refreshStatus: async () => {
    if (!isTauri() || get().operation) return;

    const requestId = ++statusRequestSequence;
    try {
      const status = await invoke<McpServiceStatus>('mcp_service_status');
      if (requestId === statusRequestSequence) {
        set({ status });
      }
    } catch (error) {
      if (requestId === statusRequestSequence) {
        set((current) => ({
          status: {
            ...current.status,
            state: 'error',
            last_error: errorMessage(error),
          },
        }));
      }
    }
  },

  startService: async () => {
    if (get().operation) return;

    statusRequestSequence += 1;
    set((current) => ({
      operation: 'start',
      status: { ...current.status, state: 'starting', last_error: null },
    }));

    try {
      const { port, authToken } = get();
      const result = await invoke<unknown>('start_mcp_service', { port, authToken });
      set({ status: requireServiceState(result, 'running') });
      notify.success(i18n.t('mcpHelper.title'), i18n.t('mcpHelper.startSuccess'));
    } catch (error) {
      const message = errorMessage(error);
      set((current) => ({
        status: { ...current.status, state: 'error', last_error: message },
      }));
      notify.error(i18n.t('mcpHelper.title'), message);
    } finally {
      set({ operation: null });
    }
  },

  stopService: async () => {
    if (get().operation) return;

    statusRequestSequence += 1;
    set((current) => ({
      operation: 'stop',
      status: { ...current.status, state: 'stopping', last_error: null },
    }));

    try {
      const result = await invoke<unknown>('stop_mcp_service');
      set({ status: requireServiceState(result, 'stopped') });
      notify.success(i18n.t('mcpHelper.title'), i18n.t('mcpHelper.stopSuccess'));
    } catch (error) {
      const message = errorMessage(error);
      set((current) => ({
        status: { ...current.status, state: 'error', last_error: message },
      }));
      notify.error(i18n.t('mcpHelper.title'), message);
    } finally {
      set({ operation: null });
    }
  },

  restartService: async () => {
    if (get().operation) return;

    statusRequestSequence += 1;
    set((current) => ({
      operation: 'restart',
      status: { ...current.status, state: 'starting', last_error: null },
    }));

    try {
      const { port, authToken } = get();
      const result = await invoke<unknown>('restart_mcp_service', { port, authToken });
      set({ status: requireServiceState(result, 'running') });
      notify.success(i18n.t('mcpHelper.title'), i18n.t('mcpHelper.restartSuccess'));
    } catch (error) {
      const message = errorMessage(error);
      set((current) => ({
        status: { ...current.status, state: 'error', last_error: message },
      }));
      notify.error(i18n.t('mcpHelper.title'), message);
    } finally {
      set({ operation: null });
    }
  },
}));
