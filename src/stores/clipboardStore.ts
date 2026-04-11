import { create } from 'zustand';
import { DbType } from '@/types/database';

interface CopiedTable {
  connectionId: string;
  database: string;
  tableName: string;
  dbType: DbType;
}

interface ClipboardStore {
  copiedTable: CopiedTable | null;
  copyTable: (info: CopiedTable) => void;
  clearClipboard: () => void;
}

export const useClipboardStore = create<ClipboardStore>((set) => ({
  copiedTable: null,
  copyTable: (info) => set({ copiedTable: info }),
  clearClipboard: () => set({ copiedTable: null }),
}));
