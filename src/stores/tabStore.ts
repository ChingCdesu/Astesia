import { create } from 'zustand';
import { TabItem } from '@/types/database';

interface TabStore {
  tabs: TabItem[];
  activeTabKey: string | null;
  addTab: (tab: TabItem) => void;
  removeTab: (key: string) => void;
  removeAllTabs: () => void;
  removeOtherTabs: (key: string) => void;
  removeLeftTabs: (key: string) => void;
  removeRightTabs: (key: string) => void;
  setActiveTab: (key: string) => void;
  updateTabContent: (key: string, content: string) => void;
}

export const useTabStore = create<TabStore>((set, get) => ({
  tabs: [],
  activeTabKey: null,

  addTab: (tab) => {
    const existing = get().tabs.find((t) => t.key === tab.key);
    if (existing) {
      set({ activeTabKey: tab.key });
    } else {
      set((state) => ({
        tabs: [...state.tabs, tab],
        activeTabKey: tab.key,
      }));
    }
  },

  removeTab: (key) =>
    set((state) => {
      const newTabs = state.tabs.filter((t) => t.key !== key);
      let newActiveKey = state.activeTabKey;
      if (state.activeTabKey === key) {
        const idx = state.tabs.findIndex((t) => t.key === key);
        newActiveKey =
          newTabs.length > 0
            ? newTabs[Math.min(idx, newTabs.length - 1)]?.key
            : null;
      }
      return { tabs: newTabs, activeTabKey: newActiveKey };
    }),

  removeAllTabs: () => set({ tabs: [], activeTabKey: null }),

  removeOtherTabs: (key) =>
    set((state) => {
      const kept = state.tabs.filter((t) => t.key === key);
      return { tabs: kept, activeTabKey: kept.length > 0 ? key : null };
    }),

  removeLeftTabs: (key) =>
    set((state) => {
      const idx = state.tabs.findIndex((t) => t.key === key);
      if (idx <= 0) return state;
      const newTabs = state.tabs.slice(idx);
      const activeStillExists = newTabs.some((t) => t.key === state.activeTabKey);
      return { tabs: newTabs, activeTabKey: activeStillExists ? state.activeTabKey : key };
    }),

  removeRightTabs: (key) =>
    set((state) => {
      const idx = state.tabs.findIndex((t) => t.key === key);
      if (idx < 0 || idx >= state.tabs.length - 1) return state;
      const newTabs = state.tabs.slice(0, idx + 1);
      const activeStillExists = newTabs.some((t) => t.key === state.activeTabKey);
      return { tabs: newTabs, activeTabKey: activeStillExists ? state.activeTabKey : key };
    }),

  setActiveTab: (key) => set({ activeTabKey: key }),

  updateTabContent: (key, content) =>
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.key === key ? { ...t, sqlContent: content } : t
      ),
    })),
}));
