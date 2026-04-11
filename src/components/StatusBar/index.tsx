import { useTranslation } from 'react-i18next';
import { useConnectionStore } from '@/stores/connectionStore';
import { DB_TYPE_LABELS } from '@/types/database';
import { cn } from '@/lib/utils';
import ThemeToggle from '@/components/ThemeToggle';
import TaskPanel from '@/components/TaskPanel';

export default function StatusBar() {
  const { t } = useTranslation();
  const { activeConnectionId, connections, treeData } = useConnectionStore();

  const activeConn = connections.find((c) => c.id === activeConnectionId);
  const isConnected = activeConnectionId ? treeData[activeConnectionId]?.connected : false;

  return (
    <div className="flex h-8 shrink-0 items-center gap-4 border-t bg-muted/40 px-4 text-xs text-muted-foreground">
      <div className="flex items-center gap-2">
        <span className={cn("h-2 w-2 rounded-full", isConnected ? "bg-emerald-500" : "bg-zinc-300")} />
        <span>
          {isConnected
            ? `${t('status.connected')}: ${activeConn?.name} (${DB_TYPE_LABELS[activeConn!.db_type]})`
            : t('status.disconnected')}
        </span>
      </div>
      <div className="ml-auto flex items-center gap-2">
        <TaskPanel />
        <ThemeToggle />
        <span>{t('status.ready')}</span>
      </div>
    </div>
  );
}
