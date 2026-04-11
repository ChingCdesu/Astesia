import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { Loader2, X, CheckCircle2, XCircle, Ban } from 'lucide-react';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Progress } from '@/components/ui/progress';
import { ScrollArea } from '@/components/ui/scroll-area';
import { useTaskStore } from '@/stores/taskStore';
import type { TaskInfo } from '@/types/task';

function TaskStatusBadge({ status }: { status: TaskInfo['status'] }) {
  const { t } = useTranslation();

  const config: Record<TaskInfo['status'], { label: string; variant: 'default' | 'secondary' | 'destructive' | 'success' | 'warning' | 'info' | 'outline' }> = {
    pending: { label: t('task.running'), variant: 'info' },
    running: { label: t('task.running'), variant: 'info' },
    completed: { label: t('task.completed'), variant: 'success' },
    failed: { label: t('task.failed'), variant: 'destructive' },
    cancelled: { label: t('task.cancelled'), variant: 'secondary' },
  };

  const { label, variant } = config[status];
  return <Badge variant={variant}>{label}</Badge>;
}

function TaskStatusIcon({ status }: { status: TaskInfo['status'] }) {
  switch (status) {
    case 'pending':
    case 'running':
      return <Loader2 className="h-4 w-4 animate-spin text-blue-500" />;
    case 'completed':
      return <CheckCircle2 className="h-4 w-4 text-emerald-500" />;
    case 'failed':
      return <XCircle className="h-4 w-4 text-destructive" />;
    case 'cancelled':
      return <Ban className="h-4 w-4 text-muted-foreground" />;
  }
}

function TaskItem({ task }: { task: TaskInfo }) {
  const { cancelTask } = useTaskStore();
  const isActive = task.status === 'running' || task.status === 'pending';

  return (
    <div className="flex flex-col gap-1.5 rounded-md border p-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <TaskStatusIcon status={task.status} />
          <span className="truncate text-sm font-medium">{task.name}</span>
        </div>
        <div className="flex items-center gap-1.5 shrink-0">
          <TaskStatusBadge status={task.status} />
          {isActive && (
            <Button
              variant="ghost"
              size="icon"
              className="h-6 w-6"
              onClick={() => cancelTask(task.id)}
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          )}
        </div>
      </div>
      {isActive && (
        <Progress value={task.progress * 100} className="h-1.5" />
      )}
      <p className="text-xs text-muted-foreground truncate">{task.message}</p>
    </div>
  );
}

export default function TaskPanel() {
  const { t } = useTranslation();
  const { tasks, initTaskListener, cleanupListeners, runningCount } = useTaskStore();
  const activeCount = runningCount();

  useEffect(() => {
    initTaskListener();
    return () => cleanupListeners();
  }, [initTaskListener, cleanupListeners]);

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="h-6 gap-1.5 px-2 text-xs text-muted-foreground"
        >
          {activeCount > 0 && (
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
          )}
          <span>
            {t('task.title')}
            {activeCount > 0 && ` (${activeCount})`}
          </span>
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" side="top" className="w-80 p-0">
        <div className="border-b px-3 py-2">
          <h4 className="text-sm font-medium">{t('task.title')}</h4>
        </div>
        <ScrollArea className="max-h-80">
          <div className="flex flex-col gap-2 p-3">
            {tasks.length === 0 ? (
              <p className="py-6 text-center text-sm text-muted-foreground">
                {t('task.noTasks')}
              </p>
            ) : (
              tasks.map((task) => <TaskItem key={task.id} task={task} />)
            )}
          </div>
        </ScrollArea>
      </PopoverContent>
    </Popover>
  );
}
