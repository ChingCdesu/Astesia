import { useTranslation } from 'react-i18next';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem,
  ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
import { ConnectionConfig, DB_TYPE_LABELS, DB_TYPE_COLORS } from '@/types/database';
import {
  ChevronRight, ChevronDown, Unplug, RefreshCw,
  Trash2, Pencil, Code, UserPlus, Database, Loader2,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { useCreateResourceStore } from '@/stores/createResourceStore';
import { DbIcon } from '@/components/ui/db-icon';

interface ConnectionNodeProps {
  conn: ConnectionConfig;
  isConnected: boolean;
  isConnecting?: boolean;
  isLoading?: boolean;
  isExpanded: boolean;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  node: any;
  onConnect: (config: ConnectionConfig) => Promise<void>;
  onToggleExpand: (key: string) => void;
  onOpenQuery: (connectionId: string, database: string) => void;
  onRefresh: (connectionId: string) => void;
  onDisconnect: (connectionId: string) => void;
  onEdit: (config: ConnectionConfig) => void;
  onDelete: (config: ConnectionConfig) => void;
}

export default function ConnectionNode({
  conn, isConnected, isConnecting, isLoading, isExpanded, node,
  onConnect, onToggleExpand, onOpenQuery,
  onRefresh, onDisconnect, onEdit, onDelete,
}: ConnectionNodeProps) {
  const { t } = useTranslation();
  const { openDialog } = useCreateResourceStore();
  const color = conn.color || DB_TYPE_COLORS[conn.db_type];
  const showSpinner = isConnecting || isLoading;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <button
          className={cn(
            "flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-sidebar-accent",
            isConnected && "font-medium",
            isConnecting && "cursor-wait opacity-70"
          )}
          disabled={isConnecting}
          onClick={async () => {
            if (isConnecting) return;
            if (!isConnected) {
              await onConnect(conn);
            } else {
              onToggleExpand(conn.id);
            }
          }}
        >
          {showSpinner ? (
            <Loader2 className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground" />
          ) : isConnected ? (
            isExpanded
              ? <ChevronDown className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              : <ChevronRight className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
          ) : (
            <span className="w-3.5 shrink-0" />
          )}
          <span
            className="h-2.5 w-2.5 shrink-0 rounded-full"
            style={{ background: isConnected ? '#22c55e' : color }}
          />
          <DbIcon dbType={conn.db_type} size={24} />
          <span className="truncate">{conn.name}</span>
          <span className="ml-auto pl-2 text-[10px] text-muted-foreground">
            {DB_TYPE_LABELS[conn.db_type]}
          </span>
        </button>
      </ContextMenuTrigger>
      <ContextMenuContent className="w-48">
        {isConnected && (
          <>
            <ContextMenuItem
              className="gap-2 py-2"
              onClick={() => onOpenQuery(conn.id, node?.databases?.[0] || '')}
            >
              <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
            </ContextMenuItem>
            <ContextMenuItem
              className="gap-2 py-2"
              onClick={() => onRefresh(conn.id)}
            >
              <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              className="gap-2 py-2"
              onClick={() => openDialog('database', conn.id, '', undefined, conn.db_type)}
            >
              <Database className="h-4 w-4" /> {t('sidebar.newDatabase')}
            </ContextMenuItem>
            <ContextMenuItem
              className="gap-2 py-2"
              onClick={() => openDialog('user', conn.id, '', undefined, conn.db_type)}
            >
              <UserPlus className="h-4 w-4" /> {t('sidebar.newUser')}
            </ContextMenuItem>
            <ContextMenuSeparator />
            <ContextMenuItem
              className="gap-2 py-2"
              onClick={() => onDisconnect(conn.id)}
            >
              <Unplug className="h-4 w-4" /> {t('sidebar.disconnect')}
            </ContextMenuItem>
            <ContextMenuSeparator />
          </>
        )}
        <ContextMenuItem
          className="gap-2 py-2"
          onClick={() => onEdit(conn)}
        >
          <Pencil className="h-4 w-4" /> {t('common.edit')}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          className="gap-2 py-2 text-destructive focus:text-destructive"
          onClick={() => onDelete(conn)}
        >
          <Trash2 className="h-4 w-4" /> {t('common.delete')}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
