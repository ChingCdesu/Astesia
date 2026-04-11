import { create } from 'zustand';
import { TabItem } from '@/types/database';

interface TabStore {
  tabs: TabItem[];
  activeTabKey: string | null;
  addTab: (tab: TabItem) => void;
  removeTab: (key: string) => void;
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

  setActiveTab: (key) => set({ activeTabKey: key }),

  updateTabContent: (key, content) =>
    set((state) => ({
      tabs: state.tabs.map((t) =>
        t.key === key ? { ...t, sqlContent: content } : t
      ),
    })),
}));
