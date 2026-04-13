import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { TaskInfo, TaskProgressEvent } from '@/types/task';

interface TaskStore {
  tasks: TaskInfo[];
  unlisteners: UnlistenFn[];

  initTaskListener: () => Promise<void>;
  cleanupListeners: () => void;
  loadTasks: () => Promise<void>;
  cancelTask: (id: string) => Promise<void>;
  runningCount: () => number;
}

export const useTaskStore = create<TaskStore>((set, get) => ({
  tasks: [],
  unlisteners: [],

  initTaskListener: async () => {
    // Cleanup any existing listeners first
    get().cleanupListeners();

    const unlistenProgress = await listen<TaskProgressEvent>('task-progress', async (event) => {
      const { id, progress, message } = event.payload;
      const exists = get().tasks.some((task) => task.id === id);
      if (!exists) {
        // Task not yet in frontend store — load from backend
        await get().loadTasks();
      }
      set((state) => ({
        tasks: state.tasks.map((task) =>
          task.id === id ? { ...task, progress, message } : task
        ),
      }));
    });

    const unlistenComplete = await listen<{ id: string }>('task-complete', async () => {
      await get().loadTasks();
    });

    set({ unlisteners: [unlistenProgress, unlistenComplete] });

    // Load initial tasks
    await get().loadTasks();
  },

  cleanupListeners: () => {
    const { unlisteners } = get();
    unlisteners.forEach((unlisten) => unlisten());
    set({ unlisteners: [] });
  },

  loadTasks: async () => {
    try {
      const tasks = await invoke<TaskInfo[]>('list_tasks');
      set({ tasks });
    } catch (e) {
      console.error('Failed to load tasks:', e);
    }
  },

  cancelTask: async (id: string) => {
    try {
      await invoke('cancel_task', { taskId: id });
      await get().loadTasks();
    } catch (e) {
      console.error('Failed to cancel task:', e);
    }
  },

  runningCount: () => {
    return get().tasks.filter((t) => t.status === 'running' || t.status === 'pending').length;
  },
}));
