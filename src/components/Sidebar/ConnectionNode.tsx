import { useTranslation } from 'react-i18next';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem,
  ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
import { ConnectionConfig, DB_TYPE_LABELS, DB_TYPE_COLORS } from '@/types/database';
import {
  ChevronRight, ChevronDown, Unplug, RefreshCw,
  Trash2, Pencil, Eye, Code, UserPlus, Database, Loader2,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { useCreateResourceStore } from '@/stores/createResourceStore';
import { DbIcon } from '@/components/ui/db-icon';
import { Badge } from '@/components/ui/badge';

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
  onEdit: (config: ConnectionConfig, readOnly: boolean) => void;
  onDelete: (config: ConnectionConfig) => void;
}

export default function ConnectionNode({
  conn, isConnected, isConnecting, isLoading, isExpanded, node,
  onConnect, onToggleExpand, onOpenQuery,
  onRefresh, onDisconnect, onEdit, onDelete,
}: ConnectionNodeProps) {
  const { t } = useTranslation();
  const { openDialog } = useCreateResourceStore();
  const isMcpInUse = conn.mcp_in_use === true;
  const isMcpDisconnecting = conn.disconnecting === true;
  const isMcpLocked = isMcpInUse || isMcpDisconnecting;
  const mcpSessionCount = conn.mcp_session_count ?? 0;
  const mcpStatusTitle = isMcpDisconnecting
    ? t('connection.mcpDisconnecting')
    : isMcpInUse
      ? t('connection.mcpInUse', { count: mcpSessionCount })
      : undefined;
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
          title={conn.last_error || mcpStatusTitle}
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
          {isMcpLocked && (
            <Badge
              variant={
                isMcpDisconnecting
                  ? 'warning'
                  : conn.mcp_connected
                    ? 'info'
                    : 'secondary'
              }
              className="ml-auto gap-1 px-1.5 py-0 text-[9px]"
              title={conn.last_error || mcpStatusTitle}
            >
              {isMcpDisconnecting && <Loader2 className="h-2.5 w-2.5 animate-spin" />}
              MCP{mcpSessionCount > 1 ? ` ${mcpSessionCount}` : ''}
            </Badge>
          )}
          <span className={cn("pl-2 text-[10px] text-muted-foreground", !isMcpLocked && "ml-auto")}>
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
          </>
        )}
        {(isConnected || isMcpLocked) && (
          <>
            {isConnected && <ContextMenuSeparator />}
            <ContextMenuItem
              disabled={isMcpDisconnecting}
              className="gap-2 py-2"
              onClick={() => onDisconnect(conn.id)}
            >
              {isMcpDisconnecting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Unplug className="h-4 w-4" />
              )}
              {t('sidebar.disconnect')}
            </ContextMenuItem>
            <ContextMenuSeparator />
          </>
        )}
        {isMcpLocked && (
          <>
            <ContextMenuItem disabled className="gap-2 py-2">
              <Database className="h-4 w-4" />
              {isMcpDisconnecting
                ? t('connection.mcpDisconnecting')
                : t('connection.mcpInUse', { count: mcpSessionCount })}
            </ContextMenuItem>
            <ContextMenuSeparator />
          </>
        )}
        <ContextMenuItem
          className="gap-2 py-2"
          onClick={() => onEdit(conn, isMcpLocked)}
        >
          {isMcpLocked ? (
            <Eye className="h-4 w-4" />
          ) : (
            <Pencil className="h-4 w-4" />
          )}
          {isMcpLocked ? t('connection.view') : t('common.edit')}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem
          disabled={isMcpLocked}
          title={isMcpLocked ? t('connection.mcpProfileLocked') : undefined}
          className="gap-2 py-2 text-destructive focus:text-destructive"
          onClick={() => onDelete(conn)}
        >
          <Trash2 className="h-4 w-4" /> {t('common.delete')}
          </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}
