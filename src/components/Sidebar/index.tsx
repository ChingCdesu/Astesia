import { useState, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Tooltip, TooltipContent, TooltipTrigger, TooltipProvider } from '@/components/ui/tooltip';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem,
  ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription,
} from '@/components/ui/dialog';
import { Label } from '@/components/ui/label';
import { useConnectionStore, loadingKey } from '@/stores/connectionStore';
import { notify } from '@/stores/notificationStore';
import { confirm } from '@/stores/confirmStore';
import { useClipboardStore } from '@/stores/clipboardStore';
import { useTabStore } from '@/stores/tabStore';
import { ConnectionConfig, DbType } from '@/types/database';
import ConnectionDialog from '../ConnectionDialog';
import CopyTableDialog from '../CopyTableDialog';
import {
  Plus, Database, ChevronRight, ChevronDown,
  Code, Eye, Users, Download, Upload,
  ClipboardPaste, RefreshCw, Zap, Trash2,
  Table2, FunctionSquare, Workflow, Layers, Loader2,
  Folder, Search,
} from 'lucide-react';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import { quoteClickHouseIdentifier } from '@/lib/sqlIdentifier';
import BackupDialog from '../BackupDialog';
import RestoreDialog from '../RestoreDialog';

import { useCreateResourceStore } from '@/stores/createResourceStore';
import ConnectionNode from './ConnectionNode';
import PostgresTree from './trees/PostgresTree';
import SqlDatabaseTree from './trees/SqlDatabaseTree';
import RedisTree from './trees/RedisTree';
import MongoTree from './trees/MongoTree';

export default function Sidebar() {
  const { t } = useTranslation();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editConfig, setEditConfig] = useState<ConnectionConfig | null>(null);
  const [dialogReadOnly, setDialogReadOnly] = useState(false);
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(new Set());
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [connectionFilter, setConnectionFilter] = useState('');
  const [backupTarget, setBackupTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [copyDialogOpen, setCopyDialogOpen] = useState(false);
  const [copySource, setCopySource] = useState<{ connectionId: string; database: string; tableName: string; dbType: DbType } | null>(null);
  const [copyTarget, setCopyTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [dragOverDbKey, setDragOverDbKey] = useState<string | null>(null);
  const [redisKeyFilter, setRedisKeyFilter] = useState<Record<string, string>>({});
  const [redisAddKeyDialog, setRedisAddKeyDialog] = useState<{ connectionId: string; database: string } | null>(null);
  const [redisNewKey, setRedisNewKey] = useState('');
  const [redisNewValue, setRedisNewValue] = useState('');
  const [redisNewTTL, setRedisNewTTL] = useState('');
  const clipboardStore = useClipboardStore();
  const { openDialog } = useCreateResourceStore();

  const {
    connections, treeData, connectDatabase, disconnectDatabase,
    removeConnection, loadTables, loadDatabases, loadSchemas,
    loadViews, loadFunctions, loadProcedures, loadTriggers, loadUsers,
    connectingIds, loadingKeys, setActiveConnection, setActiveDatabase,
  } = useConnectionStore();
  const { addTab } = useTabStore();
  const groupedConnections = useMemo(() => {
    const filter = connectionFilter.trim().toLocaleLowerCase();
    const visibleConnections = filter
      ? connections.filter((connection) => (
        connection.name.toLocaleLowerCase().includes(filter)
        || connection.db_type.toLocaleLowerCase().includes(filter)
        || connection.group_name?.toLocaleLowerCase().includes(filter)
        || connection.tags?.some((tag) => tag.toLocaleLowerCase().includes(filter))
      ))
      : connections;
    const groups = new Map<string, ConnectionConfig[]>();
    for (const connection of visibleConnections) {
      const group = connection.group_name?.trim() || '';
      groups.set(group, [...(groups.get(group) || []), connection]);
    }
    return [...groups.entries()]
      .sort(([left], [right]) => {
        if (!left) return 1;
        if (!right) return -1;
        return left.localeCompare(right);
      })
      .map(([name, items]) => ({ name, items }));
  }, [connectionFilter, connections]);

  const toggleExpand = useCallback((key: string) => {
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const handleConnect = async (config: ConnectionConfig) => {
    const result = await connectDatabase(config.id);
    if (result.success) {
      setExpandedKeys((prev) => new Set(prev).add(config.id));
    }
  };

  const handleOpenQuery = (connectionId: string, database: string) => {
    setActiveConnection(connectionId);
    setActiveDatabase(database);
    addTab({
      key: `query-${connectionId}-${database}-${Date.now()}`,
      label: `查询 [${database}]`,
      type: 'query',
      connectionId,
      database,
    });
  };

  const handleViewData = (connectionId: string, database: string, table: string) => {
    const tableParts = table.includes('.') ? table.split('.') : [table];
    const displayName = tableParts[tableParts.length - 1];
    addTab({
      key: `data-${connectionId}-${database}-${table}`,
      label: `${displayName} [${database}]`,
      type: 'table-data',
      connectionId,
      database,
      table,
    });
  };

  const handleViewStructure = (connectionId: string, database: string, table: string) => {
    const tableParts = table.includes('.') ? table.split('.') : [table];
    const displayName = tableParts[tableParts.length - 1];
    addTab({
      key: `structure-${connectionId}-${database}-${table}`,
      label: `${displayName} [结构] [${database}]`,
      type: 'table-structure',
      connectionId,
      database,
      table,
    });
  };

  const handleViewChart = (connectionId: string, database: string, table: string) => {
    const tableParts = table.includes('.') ? table.split('.') : [table];
    const displayName = tableParts[tableParts.length - 1];
    addTab({
      key: `chart-${connectionId}-${database}-${table}`,
      label: `${displayName} [图表] [${database}]`,
      type: 'data-chart',
      connectionId,
      database,
      table,
    });
  };

  const handleOpenObjectDef = (connectionId: string, database: string, objectName: string, objectType: 'view' | 'function' | 'procedure') => {
    const typeLabel = objectType === 'view' ? '视图' : objectType === 'function' ? '函数' : '存储过程';
    const nameParts = objectName.includes('.') ? objectName.split('.') : [objectName];
    const displayName = nameParts[nameParts.length - 1];
    addTab({
      key: `${objectType}-def-${connectionId}-${database}-${objectName}`,
      label: `${displayName} [${typeLabel}] [${database}]`,
      type: `${objectType}-definition` as 'view-definition' | 'function-definition' | 'procedure-definition',
      connectionId,
      database,
      table: objectName,
    });
  };

  const handleOpenERDiagram = (connectionId: string, database: string, schema?: string) => {
    const suffix = schema ? `${database}.${schema}` : database;
    addTab({
      key: `er-${connectionId}-${database}-${schema || 'all'}`,
      label: `ER 图 [${suffix}]`,
      type: 'er-diagram',
      connectionId,
      database,
      table: schema,
    });
  };

  const handleOpenPerformance = (connectionId: string, database: string) => {
    addTab({
      key: `perf-${connectionId}-${database}`,
      label: `性能 [${database}]`,
      type: 'performance',
      connectionId,
      database,
    });
  };

  const handleViewRedisKey = (connectionId: string, database: string, keyName: string) => {
    addTab({
      key: `redis-${connectionId}-${database}-${keyName}`,
      label: `${keyName} [${database}]`,
      type: 'redis-viewer',
      connectionId,
      database,
      table: keyName,
    });
  };

  const handleRedisDeleteKey = async (connectionId: string, database: string, keyName: string) => {
    const ok = await confirm(t('redis.deleteKey'), t('redis.confirmDelete'));
    if (!ok) return;
    try {
      await invoke('redis_delete_key', { connectionId, database, key: keyName });
      await loadTables(connectionId, database);
    } catch (err: unknown) {
      console.error('Failed to delete Redis key:', err);
    }
  };

  const handleRedisAddKey = async () => {
    if (!redisAddKeyDialog || !redisNewKey.trim()) return;
    const { connectionId, database } = redisAddKeyDialog;
    try {
      const ttlVal = redisNewTTL ? parseInt(redisNewTTL) : undefined;
      await invoke('redis_set_key', {
        connectionId,
        database,
        key: redisNewKey,
        value: redisNewValue || '""',
        ttl: ttlVal && ttlVal > 0 ? ttlVal : null,
      });
      await loadTables(connectionId, database);
      setRedisAddKeyDialog(null);
      setRedisNewKey('');
      setRedisNewValue('');
      setRedisNewTTL('');
    } catch (err: unknown) {
      console.error('Failed to add Redis key:', err);
    }
  };

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex h-full w-full flex-col bg-sidebar">
        {/* Header */}
        <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
          <span className="text-sm font-semibold text-sidebar-foreground">
            {t('sidebar.connections')}
          </span>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                variant="ghost"
                size="icon"
                className="h-7 w-7"
                onClick={() => { setEditConfig(null); setDialogReadOnly(false); setDialogOpen(true); }}
              >
                <Plus className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{t('sidebar.newConnection')}</TooltipContent>
          </Tooltip>
        </div>

        {connections.length > 0 && (
          <div className="relative border-b p-2">
            <Search className="pointer-events-none absolute left-4 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              className="h-8 pl-8 text-xs"
              value={connectionFilter}
              onChange={(event) => setConnectionFilter(event.target.value)}
              placeholder={t('sidebar.filterConnections')}
              aria-label={t('sidebar.filterConnections')}
            />
          </div>
        )}

        {/* Tree */}
        <ScrollArea className="flex-1">
          <div className="p-2">
            {connections.length === 0 ? (
              <div className="flex flex-col items-center gap-4 pt-20 text-muted-foreground">
                <Database className="h-12 w-12 opacity-25" />
                <p className="text-xs">暂无连接</p>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => { setEditConfig(null); setDialogReadOnly(false); setDialogOpen(true); }}
                >
                  <Plus className="mr-1.5 h-3.5 w-3.5" />
                  {t('sidebar.newConnection')}
                </Button>
              </div>
            ) : groupedConnections.length === 0 ? (
              <div className="flex flex-col items-center gap-2 pt-16 text-muted-foreground">
                <Search className="h-9 w-9 opacity-25" />
                <p className="text-xs">{t('sidebar.noMatchingConnections')}</p>
              </div>
            ) : (
              <div className="flex flex-col gap-0.5">
                {groupedConnections.map((group) => {
                  const groupKey = group.name ? `group:${group.name}` : 'group:';
                  const isGroupCollapsed = !connectionFilter.trim()
                    && collapsedGroups.has(groupKey);
                  return (
                    <div key={groupKey} className="mb-1">
                      <button
                        type="button"
                        className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-xs font-medium text-muted-foreground hover:bg-sidebar-accent hover:text-sidebar-foreground"
                        onClick={() => {
                          setCollapsedGroups((previous) => {
                            const next = new Set(previous);
                            if (next.has(groupKey)) next.delete(groupKey);
                            else next.add(groupKey);
                            return next;
                          });
                        }}
                      >
                        {isGroupCollapsed ? (
                          <ChevronRight className="h-3.5 w-3.5" />
                        ) : (
                          <ChevronDown className="h-3.5 w-3.5" />
                        )}
                        <Folder className="h-3.5 w-3.5" />
                        <span className="truncate">
                          {group.name || t('sidebar.ungrouped')}
                        </span>
                        <span className="ml-auto text-[10px] font-normal">
                          {group.items.length}
                        </span>
                      </button>
                      {!isGroupCollapsed && (
                        <div className="ml-2 border-l pl-1">
                          {group.items.map((conn) => {
                  const node = treeData[conn.id];
                  const isConnected = node?.connected;
                  const isExpanded = expandedKeys.has(conn.id);

                  return (
                    <div key={conn.id}>
                      {/* Connection Node */}
                      <ConnectionNode
                        conn={conn}
                        isConnected={!!isConnected}
                        isConnecting={connectingIds.has(conn.id)}
                        isLoading={loadingKeys.has(loadingKey.databases(conn.id))}
                        isExpanded={isExpanded}
                        node={node}
                        onConnect={handleConnect}
                        onToggleExpand={toggleExpand}
                        onOpenQuery={handleOpenQuery}
                        onRefresh={loadDatabases}
                        onDisconnect={disconnectDatabase}
                        onEdit={(c, readOnly) => {
                          setEditConfig(c);
                          setDialogReadOnly(readOnly);
                          setDialogOpen(true);
                        }}
                        onDelete={(c) => {
                          void (async () => {
                            try {
                              const approved = await confirm(
                                t('sidebar.deleteConnection'),
                                t('sidebar.confirmDeleteConnection', { name: c.name })
                              );
                              if (!approved) return;
                              await removeConnection(c.id);
                            } catch (error) {
                              const repositoryError = error as {
                                message?: string;
                                remediation?: string;
                              };
                              notify.error(
                                '删除连接失败',
                                [
                                  repositoryError?.message || String(error),
                                  repositoryError?.remediation,
                                ].filter(Boolean).join(' ')
                              );
                            }
                          })();
                        }}
                      />

                      {/* Databases */}
                      {isConnected && isExpanded && (node?.databases || []).map((db) => {
                        const dbKey = `${conn.id}::${db}`;
                        const dbExpanded = expandedKeys.has(dbKey);

                        const isPG = conn.db_type === 'postgresql';
                        const isRedis = conn.db_type === 'redis';
                        const isMongo = conn.db_type === 'mongodb';
                        const isSQL = !isRedis && !isMongo;

                        const dbLoadingKey = isPG
                          ? loadingKey.schemas(conn.id, db)
                          : loadingKey.tables(conn.id, db);
                        const dbLoading = loadingKeys.has(dbLoadingKey);

                        return (
                          <div key={dbKey}>
                            {/* Database node with context menu */}
                            <ContextMenu>
                              <ContextMenuTrigger asChild>
                                <button
                                  className={cn(
                                    "flex w-full items-center gap-2 rounded-md py-1.5 pl-8 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent",
                                    dragOverDbKey === dbKey && "ring-2 ring-primary bg-sidebar-accent"
                                  )}
                                  onClick={async () => {
                                    setActiveConnection(conn.id);
                                    setActiveDatabase(db);
                                    toggleExpand(dbKey);
                                    if (!dbExpanded) {
                                      if (isPG) {
                                        await loadSchemas(conn.id, db);
                                      } else {
                                        await loadTables(conn.id, db);
                                      }
                                    }
                                  }}
                                  onDragOver={(e) => {
                                    e.preventDefault();
                                    e.dataTransfer.dropEffect = 'copy';
                                    setDragOverDbKey(dbKey);
                                  }}
                                  onDragLeave={() => {
                                    setDragOverDbKey(null);
                                  }}
                                  onDrop={(e) => {
                                    e.preventDefault();
                                    setDragOverDbKey(null);
                                    try {
                                      const source = JSON.parse(e.dataTransfer.getData('application/json'));
                                      if (source.dbType === conn.db_type) {
                                        setCopySource(source);
                                        setCopyTarget({ connectionId: conn.id, database: db });
                                        setCopyDialogOpen(true);
                                      }
                                    } catch { /* ignore */ }
                                  }}
                                >
                                  {dbLoading ? (
                                    <Loader2 className="h-3 w-3 shrink-0 animate-spin text-muted-foreground" />
                                  ) : dbExpanded ? (
                                    <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                                  ) : (
                                    <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                                  )}
                                  <Database className="h-3.5 w-3.5 shrink-0 text-blue-500" />
                                  <span className="truncate">{db}</span>
                                </button>
                              </ContextMenuTrigger>
                              <ContextMenuContent className="w-44">
                                {isRedis ? (
                                  <>
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => setRedisAddKeyDialog({ connectionId: conn.id, database: db })}>
                                      <Plus className="h-4 w-4" /> {t('redis.addKey')}
                                    </ContextMenuItem>
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenQuery(conn.id, db)}>
                                      <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
                                    </ContextMenuItem>
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenPerformance(conn.id, db)}>
                                      <Zap className="h-4 w-4" /> {t('sidebar.performance')}
                                    </ContextMenuItem>
                                    <ContextMenuSeparator />
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => loadTables(conn.id, db)}>
                                      <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                                    </ContextMenuItem>
                                  </>
                                ) : (
                                  <>
                                    {isPG ? (
                                      <>
                                        <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('schema', conn.id, db, undefined, conn.db_type)}>
                                          <Layers className="h-4 w-4" /> {t('sidebar.newSchema')}
                                        </ContextMenuItem>
                                        <ContextMenuSeparator />
                                      </>
                                    ) : (
                                      <>
                                        <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('table', conn.id, db, undefined, conn.db_type)}>
                                          <Table2 className="h-4 w-4" /> {t('sidebar.newTable')}
                                        </ContextMenuItem>
                                        <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('view', conn.id, db, undefined, conn.db_type)}>
                                          <Eye className="h-4 w-4" /> {t('sidebar.newView')}
                                        </ContextMenuItem>
                                        <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('function', conn.id, db, undefined, conn.db_type)}>
                                          <FunctionSquare className="h-4 w-4" /> {t('sidebar.newFunction')}
                                        </ContextMenuItem>
                                        {conn.db_type !== 'clickhouse' && (
                                          <>
                                            <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('procedure', conn.id, db, undefined, conn.db_type)}>
                                              <Workflow className="h-4 w-4" /> {t('sidebar.newProcedure')}
                                            </ContextMenuItem>
                                            <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('trigger', conn.id, db, undefined, conn.db_type)}>
                                              <Zap className="h-4 w-4" /> {t('sidebar.newTrigger')}
                                            </ContextMenuItem>
                                          </>
                                        )}
                                        <ContextMenuSeparator />
                                      </>
                                    )}
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenQuery(conn.id, db)}>
                                      <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
                                    </ContextMenuItem>
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenERDiagram(conn.id, db)}>
                                      <Eye className="h-4 w-4" /> {t('sidebar.erDiagram')}
                                    </ContextMenuItem>
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenPerformance(conn.id, db)}>
                                      <Zap className="h-4 w-4" /> {t('sidebar.performance')}
                                    </ContextMenuItem>
                                    <ContextMenuSeparator />
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => setBackupTarget({ connectionId: conn.id, database: db })}>
                                      <Download className="h-4 w-4" /> {t('backup.title')}
                                    </ContextMenuItem>
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => setRestoreTarget({ connectionId: conn.id, database: db })}>
                                      <Upload className="h-4 w-4" /> {t('backup.restore')}
                                    </ContextMenuItem>
                                    {clipboardStore.copiedTable && clipboardStore.copiedTable.dbType === conn.db_type && (
                                      <>
                                        <ContextMenuSeparator />
                                        <ContextMenuItem className="gap-2 py-2" onClick={() => {
                                          setCopySource(clipboardStore.copiedTable!);
                                          setCopyTarget({ connectionId: conn.id, database: db });
                                          setCopyDialogOpen(true);
                                        }}>
                                          <ClipboardPaste className="h-4 w-4" /> {t('tableCopy.pasteTable')}
                                        </ContextMenuItem>
                                      </>
                                    )}
                                    <ContextMenuSeparator />
                                    <ContextMenuItem className="gap-2 py-2" onClick={() => {
                                      if (isPG) loadSchemas(conn.id, db);
                                      else loadTables(conn.id, db);
                                    }}>
                                      <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                                    </ContextMenuItem>
                                    <ContextMenuSeparator />
                                    <ContextMenuItem
                                      className="gap-2 py-2 text-destructive focus:text-destructive"
                                      onClick={async () => {
                                        const ok = await confirm('删除数据库', `确认删除数据库 "${db}" 吗？此操作不可恢复。`);
                                        if (!ok) return;
                                        try {
                                          if (conn.db_type === 'postgresql') {
                                            // PG: must disconnect all sessions from target DB first, then DROP from a different DB
                                            const adminDb = conn.database || 'postgres';
                                            const safeDb = adminDb === db ? 'postgres' : adminDb;
                                            // Terminate all connections to the target database
                                            await invoke('execute_query', {
                                              connectionId: conn.id,
                                              database: safeDb,
                                              sql: `SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${db}' AND pid <> pg_backend_pid()`,
                                            });
                                            // Now drop
                                            await invoke('execute_query', {
                                              connectionId: conn.id,
                                              database: safeDb,
                                              sql: `DROP DATABASE "${db}"`,
                                            });
                                          } else {
                                            const sql = conn.db_type === 'mysql'
                                              ? `DROP DATABASE \`${db}\``
                                              : conn.db_type === 'clickhouse'
                                                ? `DROP DATABASE ${quoteClickHouseIdentifier(db)}`
                                              : conn.db_type === 'sqlserver'
                                                ? `DROP DATABASE [${db}]`
                                                : `DROP DATABASE "${db}"`;
                                            await invoke('execute_query', { connectionId: conn.id, database: db, sql });
                                          }
                                          notify.success('数据库已删除', db);
                                          loadDatabases(conn.id);
                                        } catch (e: unknown) {
                                          notify.error('删除数据库失败', String(e));
                                        }
                                      }}
                                    >
                                      <Trash2 className="h-4 w-4" /> 删除数据库
                                    </ContextMenuItem>
                                  </>
                                )}
                              </ContextMenuContent>
                            </ContextMenu>

                            {/* Database tree content */}
                            {dbExpanded && isPG && (
                              <PostgresTree
                                conn={conn}
                                node={node}
                                db={db}
                                dbKey={dbKey}
                                expandedKeys={expandedKeys}
                                toggleExpand={toggleExpand}
                                handleViewData={handleViewData}
                                handleViewStructure={handleViewStructure}
                                handleViewChart={handleViewChart}
                                handleOpenObjectDef={handleOpenObjectDef}
                                handleOpenQuery={handleOpenQuery}
                                loadTables={loadTables}
                                loadViews={loadViews}
                                loadFunctions={loadFunctions}
                                loadProcedures={loadProcedures}
                                loadTriggers={loadTriggers}
                                loadSchemas={loadSchemas}
                                handleOpenERDiagram={handleOpenERDiagram}
                                clipboardStore={clipboardStore}
                              />
                            )}

                            {dbExpanded && isRedis && (
                              <RedisTree
                                conn={conn}
                                node={node}
                                db={db}
                                dbKey={dbKey}
                                expandedKeys={expandedKeys}
                                toggleExpand={toggleExpand}
                                handleViewRedisKey={handleViewRedisKey}
                                handleRedisDeleteKey={handleRedisDeleteKey}
                                redisKeyFilter={redisKeyFilter}
                                setRedisKeyFilter={setRedisKeyFilter}
                                setRedisAddKeyDialog={setRedisAddKeyDialog}
                                loadTables={loadTables}
                              />
                            )}

                            {dbExpanded && isMongo && (
                              <MongoTree
                                conn={conn}
                                node={node}
                                db={db}
                                dbKey={dbKey}
                                expandedKeys={expandedKeys}
                                toggleExpand={toggleExpand}
                                addTab={addTab}
                                loadTables={loadTables}
                              />
                            )}

                            {dbExpanded && isSQL && !isPG && (
                              <SqlDatabaseTree
                                conn={conn}
                                node={node}
                                db={db}
                                dbKey={dbKey}
                                expandedKeys={expandedKeys}
                                toggleExpand={toggleExpand}
                                handleViewData={handleViewData}
                                handleViewStructure={handleViewStructure}
                                handleViewChart={handleViewChart}
                                handleOpenObjectDef={handleOpenObjectDef}
                                handleOpenQuery={handleOpenQuery}
                                loadTables={loadTables}
                                loadViews={loadViews}
                                loadFunctions={loadFunctions}
                                loadProcedures={loadProcedures}
                                loadTriggers={loadTriggers}
                                clipboardStore={clipboardStore}
                              />
                            )}
                          </div>
                        );
                      })}

                      {/* Users Node at connection level */}
                      {isConnected && isExpanded && (() => {
                        const usersKey = `${conn.id}::users`;
                        const usersExpanded = expandedKeys.has(usersKey);
                        const users = node?.users || [];

                        return (
                          <div>
                            <button
                              className="flex w-full items-center gap-2 rounded-md py-1.5 pl-8 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
                              onClick={() => {
                                toggleExpand(usersKey);
                                if (!usersExpanded) loadUsers(conn.id);
                              }}
                            >
                              {usersExpanded
                                ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                                : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                              }
                              <Users className="h-3.5 w-3.5 shrink-0 text-gray-500" />
                              <span className="text-muted-foreground">{t('sidebar.users')} ({users.length})</span>
                            </button>
                            {usersExpanded && users.map((user) => (
                              <Tooltip key={`${conn.id}::user::${user.name}${user.host || ''}`}>
                                <TooltipTrigger asChild>
                                  <button
                                    className="flex w-full items-center gap-2 rounded-md py-1.5 pl-14 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
                                  >
                                    <Users className="h-3.5 w-3.5 shrink-0 text-gray-500" />
                                    <span className="truncate">{user.name}</span>
                                    {user.host && (
                                      <Badge variant={user.host === 'user' ? 'info' : 'secondary'} className="ml-auto text-[9px] px-1 py-0">
                                        {user.host === 'user' ? '用户' : '组'}
                                      </Badge>
                                    )}
                                  </button>
                                </TooltipTrigger>
                                <TooltipContent side="right">
                                  {user.name}{user.host ? ` (${user.host === 'user' ? '用户' : '组'})` : ''}
                                </TooltipContent>
                              </Tooltip>
                            ))}
                          </div>
                        );
                      })()}
                            </div>
                          );
                        })}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </ScrollArea>

        <ConnectionDialog
          open={dialogOpen}
          onClose={() => {
            setDialogOpen(false);
            setDialogReadOnly(false);
          }}
          editConfig={editConfig}
          readOnly={dialogReadOnly || Boolean(editConfig && (
            connections.find((connection) => connection.id === editConfig.id)?.mcp_in_use
            || connections.find((connection) => connection.id === editConfig.id)?.disconnecting
          ))}
        />

        <BackupDialog
          open={!!backupTarget}
          onClose={() => setBackupTarget(null)}
          connectionId={backupTarget?.connectionId || ''}
          database={backupTarget?.database || ''}
        />

        <RestoreDialog
          open={!!restoreTarget}
          onClose={() => setRestoreTarget(null)}
          connectionId={restoreTarget?.connectionId || ''}
          database={restoreTarget?.database || ''}
        />

        {copySource && copyTarget && (
          <CopyTableDialog
            open={copyDialogOpen}
            onClose={() => { setCopyDialogOpen(false); setCopySource(null); setCopyTarget(null); }}
            source={copySource}
            target={copyTarget}
          />
        )}

        {/* Redis Add Key Dialog */}
        <Dialog open={!!redisAddKeyDialog} onOpenChange={(open) => { if (!open) { setRedisAddKeyDialog(null); setRedisNewKey(''); setRedisNewValue(''); setRedisNewTTL(''); } }}>
          <DialogContent className="sm:max-w-[400px]">
            <DialogHeader>
              <DialogTitle>{t('redis.addKey')}</DialogTitle>
              <DialogDescription>{redisAddKeyDialog?.database}</DialogDescription>
            </DialogHeader>
            <div className="flex flex-col gap-3 py-2">
              <div>
                <Label className="mb-1 block text-sm">{t('redis.keyName')}</Label>
                <Input
                  value={redisNewKey}
                  onChange={(e) => setRedisNewKey(e.target.value)}
                  placeholder={t('redis.keyName')}
                />
              </div>
              <div>
                <Label className="mb-1 block text-sm">{t('redis.value')}</Label>
                <Input
                  value={redisNewValue}
                  onChange={(e) => setRedisNewValue(e.target.value)}
                  placeholder={t('redis.value')}
                />
              </div>
              <div>
                <Label className="mb-1 block text-sm">{t('redis.ttl')}</Label>
                <Input
                  type="number"
                  value={redisNewTTL}
                  onChange={(e) => setRedisNewTTL(e.target.value)}
                  placeholder={t('redis.noTTL')}
                />
              </div>
            </div>
            <DialogFooter>
              <Button variant="outline" onClick={() => { setRedisAddKeyDialog(null); setRedisNewKey(''); setRedisNewValue(''); setRedisNewTTL(''); }}>
                {t('common.cancel')}
              </Button>
              <Button onClick={handleRedisAddKey} disabled={!redisNewKey.trim()}>
                {t('common.confirm')}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </TooltipProvider>
  );
}
