import { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Button } from '@/components/ui/button';
import { ScrollArea } from '@/components/ui/scroll-area';
import { Tooltip, TooltipContent, TooltipTrigger, TooltipProvider } from '@/components/ui/tooltip';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem,
  ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
import { useConnectionStore } from '@/stores/connectionStore';
import { useClipboardStore } from '@/stores/clipboardStore';
import { useTabStore } from '@/stores/tabStore';
import { ConnectionConfig, DB_TYPE_LABELS, DB_TYPE_COLORS, DbType } from '@/types/database';
import ConnectionDialog from '../ConnectionDialog';
import CopyTableDialog from '../CopyTableDialog';
import {
  Plus, Database, Table2, ChevronRight, ChevronDown,
  Unplug, RefreshCw, Trash2, Pencil, Code, Eye, Columns,
  FunctionSquare, Workflow, Zap, Users, Download, Upload,
  Copy, ClipboardPaste, BarChart3,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import BackupDialog from '../BackupDialog';
import RestoreDialog from '../RestoreDialog';

export default function Sidebar() {
  const { t } = useTranslation();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [editConfig, setEditConfig] = useState<ConnectionConfig | null>(null);
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(new Set());
  const [backupTarget, setBackupTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [copyDialogOpen, setCopyDialogOpen] = useState(false);
  const [copySource, setCopySource] = useState<{ connectionId: string; database: string; tableName: string; dbType: DbType } | null>(null);
  const [copyTarget, setCopyTarget] = useState<{ connectionId: string; database: string } | null>(null);
  const [dragOverDbKey, setDragOverDbKey] = useState<string | null>(null);

  const clipboardStore = useClipboardStore();

  const {
    connections, treeData, connectDatabase, disconnectDatabase,
    removeConnection, loadTables, loadDatabases,
    loadViews, loadFunctions, loadProcedures, loadTriggers, loadUsers,
  } = useConnectionStore();
  const { addTab } = useTabStore();

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
    addTab({
      key: `query-${connectionId}-${database}-${Date.now()}`,
      label: `查询 - ${database}`,
      type: 'query',
      connectionId,
      database,
    });
  };

  const handleViewData = (connectionId: string, database: string, table: string) => {
    addTab({
      key: `data-${connectionId}-${database}-${table}`,
      label: table,
      type: 'table-data',
      connectionId,
      database,
      table,
    });
  };

  const handleViewStructure = (connectionId: string, database: string, table: string) => {
    addTab({
      key: `structure-${connectionId}-${database}-${table}`,
      label: `${table} [结构]`,
      type: 'table-structure',
      connectionId,
      database,
      table,
    });
  };

  const handleViewChart = (connectionId: string, database: string, table: string) => {
    addTab({
      key: `chart-${connectionId}-${database}-${table}`,
      label: `${table} [图表]`,
      type: 'data-chart',
      connectionId,
      database,
      table,
    });
  };

  const handleOpenObjectDef = (connectionId: string, database: string, objectName: string, objectType: 'view' | 'function' | 'procedure') => {
    const typeLabel = objectType === 'view' ? '视图' : objectType === 'function' ? '函数' : '存储过程';
    addTab({
      key: `${objectType}-def-${connectionId}-${database}-${objectName}`,
      label: `${objectName} [${typeLabel}]`,
      type: `${objectType}-definition` as 'view-definition' | 'function-definition' | 'procedure-definition',
      connectionId,
      database,
      table: objectName,
    });
  };

  const handleOpenERDiagram = (connectionId: string, database: string) => {
    addTab({
      key: `er-${connectionId}-${database}`,
      label: `ER 图 - ${database}`,
      type: 'er-diagram',
      connectionId,
      database,
    });
  };

  const handleOpenPerformance = (connectionId: string, database: string) => {
    addTab({
      key: `perf-${connectionId}-${database}`,
      label: `性能 - ${database}`,
      type: 'performance',
      connectionId,
      database,
    });
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
                onClick={() => { setEditConfig(null); setDialogOpen(true); }}
              >
                <Plus className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent side="bottom">{t('sidebar.newConnection')}</TooltipContent>
          </Tooltip>
        </div>

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
                  onClick={() => { setEditConfig(null); setDialogOpen(true); }}
                >
                  <Plus className="mr-1.5 h-3.5 w-3.5" />
                  {t('sidebar.newConnection')}
                </Button>
              </div>
            ) : (
              <div className="flex flex-col gap-0.5">
                {connections.map((conn) => {
                  const node = treeData[conn.id];
                  const isConnected = node?.connected;
                  const color = conn.color || DB_TYPE_COLORS[conn.db_type];
                  const isExpanded = expandedKeys.has(conn.id);

                  return (
                    <div key={conn.id}>
                      {/* Connection Node */}
                      <ContextMenu>
                        <ContextMenuTrigger asChild>
                          <button
                            className={cn(
                              "flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-sidebar-accent",
                              isConnected && "font-medium"
                            )}
                            onClick={async () => {
                              if (!isConnected) {
                                await handleConnect(conn);
                              } else {
                                toggleExpand(conn.id);
                              }
                            }}
                          >
                            {isConnected ? (
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
                            <Database className="h-4 w-4 shrink-0" style={{ color }} />
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
                                onClick={() => handleOpenQuery(conn.id, node?.databases?.[0] || '')}
                              >
                                <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
                              </ContextMenuItem>
                              <ContextMenuItem
                                className="gap-2 py-2"
                                onClick={() => loadDatabases(conn.id)}
                              >
                                <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                              </ContextMenuItem>
                              <ContextMenuSeparator />
                              <ContextMenuItem
                                className="gap-2 py-2"
                                onClick={() => disconnectDatabase(conn.id)}
                              >
                                <Unplug className="h-4 w-4" /> {t('sidebar.disconnect')}
                              </ContextMenuItem>
                              <ContextMenuSeparator />
                            </>
                          )}
                          <ContextMenuItem
                            className="gap-2 py-2"
                            onClick={() => { setEditConfig(conn); setDialogOpen(true); }}
                          >
                            <Pencil className="h-4 w-4" /> {t('common.edit')}
                          </ContextMenuItem>
                          <ContextMenuSeparator />
                          <ContextMenuItem
                            className="gap-2 py-2 text-destructive focus:text-destructive"
                            onClick={() => {
                              if (treeData[conn.id]?.connected) disconnectDatabase(conn.id);
                              removeConnection(conn.id);
                            }}
                          >
                            <Trash2 className="h-4 w-4" /> {t('common.delete')}
                          </ContextMenuItem>
                        </ContextMenuContent>
                      </ContextMenu>

                      {/* Databases */}
                      {isConnected && isExpanded && (node?.databases || []).map((db) => {
                        const dbKey = `${conn.id}::${db}`;
                        const dbExpanded = expandedKeys.has(dbKey);

                        const tables = node?.tables?.[db] || [];
                        const views = node?.views?.[db] || [];
                        const functions = node?.functions?.[db] || [];
                        const procedures = node?.procedures?.[db] || [];
                        const triggers = node?.triggers?.[db] || [];

                        const tablesKey = `${dbKey}::tables`;
                        const viewsKey = `${dbKey}::views`;
                        const functionsKey = `${dbKey}::functions`;
                        const proceduresKey = `${dbKey}::procedures`;
                        const triggersKey = `${dbKey}::triggers`;

                        const tablesExpanded = expandedKeys.has(tablesKey);
                        const viewsExpanded = expandedKeys.has(viewsKey);
                        const functionsExpanded = expandedKeys.has(functionsKey);
                        const proceduresExpanded = expandedKeys.has(proceduresKey);
                        const triggersExpanded = expandedKeys.has(triggersKey);

                        // Redis/MongoDB don't have SQL object categories
                        const isSQL = conn.db_type !== 'redis' && conn.db_type !== 'mongodb';
                        const isRedis = conn.db_type === 'redis';
                        const isMongo = conn.db_type === 'mongodb';
                        const tableLabel = isRedis ? '键' : isMongo ? '集合' : t('sidebar.tables');

                        return (
                          <div key={dbKey}>
                            <ContextMenu>
                              <ContextMenuTrigger asChild>
                                <button
                                  className={cn(
                                    "flex w-full items-center gap-2 rounded-md py-1.5 pl-8 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent",
                                    dragOverDbKey === dbKey && "ring-2 ring-primary bg-sidebar-accent"
                                  )}
                                  onClick={async () => {
                                    toggleExpand(dbKey);
                                    if (!dbExpanded) {
                                      await loadTables(conn.id, db);
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
                                  {dbExpanded
                                    ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                                    : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                                  }
                                  <Database className="h-3.5 w-3.5 shrink-0 text-blue-500" />
                                  <span className="truncate">{db}</span>
                                </button>
                              </ContextMenuTrigger>
                              <ContextMenuContent className="w-44">
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
                                <ContextMenuItem className="gap-2 py-2" onClick={() => loadTables(conn.id, db)}>
                                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                                </ContextMenuItem>
                              </ContextMenuContent>
                            </ContextMenu>

                            {dbExpanded && (
                              <>
                                {/* Tables Category */}
                                <button
                                  className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                                  onClick={() => {
                                    toggleExpand(tablesKey);
                                    if (!tablesExpanded) loadTables(conn.id, db);
                                  }}
                                >
                                  {tablesExpanded
                                    ? <ChevronDown className="h-2.5 w-2.5 shrink-0" />
                                    : <ChevronRight className="h-2.5 w-2.5 shrink-0" />
                                  }
                                  <Table2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                                  <span>{tableLabel} ({tables.length})</span>
                                </button>
                                {tablesExpanded && tables.map((table) => (
                                  <ContextMenu key={`${dbKey}::table::${table.name}`}>
                                    <ContextMenuTrigger asChild>
                                      <button
                                        className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
                                        onClick={() => handleViewData(conn.id, db, table.name)}
                                        draggable
                                        onDragStart={(e) => {
                                          e.dataTransfer.setData('application/json', JSON.stringify({
                                            connectionId: conn.id,
                                            database: db,
                                            tableName: table.name,
                                            dbType: conn.db_type,
                                          }));
                                          e.dataTransfer.effectAllowed = 'copy';
                                        }}
                                      >
                                        <Table2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                                        <span className="truncate">{table.name}</span>
                                      </button>
                                    </ContextMenuTrigger>
                                    <ContextMenuContent className="w-44">
                                      <ContextMenuItem className="gap-2 py-2" onClick={() => handleViewData(conn.id, db, table.name)}>
                                        <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                                      </ContextMenuItem>
                                      <ContextMenuItem className="gap-2 py-2" onClick={() => handleViewStructure(conn.id, db, table.name)}>
                                        <Columns className="h-4 w-4" /> {t('sidebar.viewStructure')}
                                      </ContextMenuItem>
                                      <ContextMenuItem className="gap-2 py-2" onClick={() => handleViewChart(conn.id, db, table.name)}>
                                        <BarChart3 className="h-4 w-4" /> {t('chart.title')}
                                      </ContextMenuItem>
                                      <ContextMenuSeparator />
                                      <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenQuery(conn.id, db)}>
                                        <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
                                      </ContextMenuItem>
                                      <ContextMenuSeparator />
                                      <ContextMenuItem className="gap-2 py-2" onClick={() => {
                                        clipboardStore.copyTable({ connectionId: conn.id, database: db, tableName: table.name, dbType: conn.db_type });
                                      }}>
                                        <Copy className="h-4 w-4" /> {t('tableCopy.copyTable')}
                                      </ContextMenuItem>
                                    </ContextMenuContent>
                                  </ContextMenu>
                                ))}

                                {/* Views/Functions/Procedures/Triggers — SQL databases only */}
                                {isSQL && (
                                  <>
                                    {/* Views */}
                                    <button
                                      className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                                      onClick={() => { toggleExpand(viewsKey); if (!viewsExpanded) loadViews(conn.id, db); }}
                                    >
                                      {viewsExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                                      <Eye className="h-3.5 w-3.5 shrink-0 text-blue-500" />
                                      <span>{t('sidebar.views')} ({views.length})</span>
                                    </button>
                                    {viewsExpanded && views.map((view) => (
                                      <button key={`${dbKey}::view::${view.name}`} className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, view.name, 'view')}>
                                        <Eye className="h-3.5 w-3.5 shrink-0 text-blue-500" /><span className="truncate">{view.name}</span>
                                      </button>
                                    ))}

                                    {/* Functions */}
                                    <button
                                      className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                                      onClick={() => { toggleExpand(functionsKey); if (!functionsExpanded) loadFunctions(conn.id, db); }}
                                    >
                                      {functionsExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                                      <FunctionSquare className="h-3.5 w-3.5 shrink-0 text-purple-500" />
                                      <span>{t('sidebar.functions')} ({functions.length})</span>
                                    </button>
                                    {functionsExpanded && functions.map((func) => (
                                      <button key={`${dbKey}::func::${func.name}`} className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, func.name, 'function')}>
                                        <FunctionSquare className="h-3.5 w-3.5 shrink-0 text-purple-500" /><span className="truncate">{func.name}</span>
                                      </button>
                                    ))}

                                    {/* Procedures */}
                                    <button
                                      className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                                      onClick={() => { toggleExpand(proceduresKey); if (!proceduresExpanded) loadProcedures(conn.id, db); }}
                                    >
                                      {proceduresExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                                      <Workflow className="h-3.5 w-3.5 shrink-0 text-orange-500" />
                                      <span>{t('sidebar.procedures')} ({procedures.length})</span>
                                    </button>
                                    {proceduresExpanded && procedures.map((proc) => (
                                      <button key={`${dbKey}::proc::${proc.name}`} className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, proc.name, 'procedure')}>
                                        <Workflow className="h-3.5 w-3.5 shrink-0 text-orange-500" /><span className="truncate">{proc.name}</span>
                                      </button>
                                    ))}

                                    {/* Triggers */}
                                    <button
                                      className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                                      onClick={() => { toggleExpand(triggersKey); if (!triggersExpanded) loadTriggers(conn.id, db); }}
                                    >
                                      {triggersExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                                      <Zap className="h-3.5 w-3.5 shrink-0 text-yellow-500" />
                                      <span>{t('sidebar.triggers')} ({triggers.length})</span>
                                    </button>
                                    {triggersExpanded && triggers.map((trigger) => (
                                      <Tooltip key={`${dbKey}::trigger::${trigger.name}`}>
                                        <TooltipTrigger asChild>
                                          <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent">
                                            <Zap className="h-3.5 w-3.5 shrink-0 text-yellow-500" /><span className="truncate">{trigger.name}</span>
                                          </button>
                                        </TooltipTrigger>
                                        <TooltipContent side="right">{trigger.timing} {trigger.event} ON {trigger.table}</TooltipContent>
                                      </Tooltip>
                                    ))}
                                  </>
                                )}
                              </>
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
                                      <span className="ml-auto text-[10px] text-muted-foreground">@{user.host}</span>
                                    )}
                                  </button>
                                </TooltipTrigger>
                                <TooltipContent side="right">
                                  {user.name}{user.host ? `@${user.host}` : ''}
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
        </ScrollArea>

        <ConnectionDialog
          open={dialogOpen}
          onClose={() => setDialogOpen(false)}
          editConfig={editConfig}
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
      </div>
    </TooltipProvider>
  );
}
