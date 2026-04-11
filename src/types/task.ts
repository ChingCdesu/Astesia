export interface TaskInfo {
  id: string;
  name: string;
  status: 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
  progress: number;
  message: string;
  created_at: string;
  completed_at: string | null;
}

export interface TaskProgressEvent {
  id: string;
  progress: number;
  message: string;
}
