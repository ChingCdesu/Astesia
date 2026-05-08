import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem,
  ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
import {
  Table2, ChevronRight, ChevronDown, Eye, Columns, Code,
  FunctionSquare, Workflow, Zap, Copy, Layers, BarChart3,
  Pencil, Trash2, GitBranch, RefreshCw, Loader2,
} from 'lucide-react';
import type { PostgresTreeProps } from './types';
import { VirtualList } from './VirtualList';
import { useCreateResourceStore } from '@/stores/createResourceStore';
import { notify } from '@/stores/notificationStore';
import { confirm } from '@/stores/confirmStore';
import { useConnectionStore, loadingKey } from '@/stores/connectionStore';
import {
  getDropTableSQL, getRenameTableSQL, getDropViewSQL,
  getDropFunctionSQL, getDropProcedureSQL, getDropTriggerSQL,
  getDropSchemaSQL, getRenameSchemaSQL,
} from './sqlHelpers';

export default function PostgresTree({
  conn, node, db, dbKey, expandedKeys, toggleExpand,
  handleViewData, handleViewStructure, handleViewChart,
  handleOpenObjectDef, handleOpenQuery,
  loadTables, loadViews, loadFunctions, loadProcedures, loadTriggers,
  loadSchemas,
  handleOpenERDiagram,
  clipboardStore,
}: PostgresTreeProps) {
  const { t } = useTranslation();
  const { openDialog } = useCreateResourceStore();
  const loadingKeys = useConnectionStore((s) => s.loadingKeys);
  const tablesLoading = loadingKeys.has(loadingKey.tables(conn.id, db));
  const viewsLoading = loadingKeys.has(loadingKey.views(conn.id, db));
  const functionsLoading = loadingKeys.has(loadingKey.functions(conn.id, db));
  const proceduresLoading = loadingKeys.has(loadingKey.procedures(conn.id, db));
  const triggersLoading = loadingKeys.has(loadingKey.triggers(conn.id, db));

  const handleRenameTable = async (qualifiedName: string) => {
    const parts = qualifiedName.split('.');
    const tableName = parts.length === 2 ? parts[1] : qualifiedName;
    const newName = window.prompt(t('sidebar.renameTable'), tableName);
    if (!newName || newName === tableName) return;
    try {
      const sql = getRenameTableSQL(qualifiedName, newName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadTables(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropTable = async (qualifiedName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: qualifiedName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropTableSQL(qualifiedName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadTables(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropView = async (qualifiedName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: qualifiedName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropViewSQL(qualifiedName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadViews(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropFunction = async (qualifiedName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: qualifiedName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropFunctionSQL(qualifiedName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadFunctions(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropProcedure = async (qualifiedName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: qualifiedName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropProcedureSQL(qualifiedName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadProcedures(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropTrigger = async (qualifiedName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: qualifiedName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropTriggerSQL(qualifiedName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadTriggers(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleRenameSchema = async (schemaName: string) => {
    const newName = window.prompt(t('sidebar.renameSchema'), schemaName);
    if (!newName || newName === schemaName) return;
    try {
      const sql = getRenameSchemaSQL(schemaName, newName);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadSchemas(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropSchema = async (schemaName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: schemaName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropSchemaSQL(schemaName);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadSchemas(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const tables = node?.tables?.[db] || [];
  const views = node?.views?.[db] || [];
  const functions = node?.functions?.[db] || [];
  const procedures = node?.procedures?.[db] || [];
  const triggers = node?.triggers?.[db] || [];
  const schemas = node?.schemas?.[db] || [];

  return (
    <>
      {schemas.map((schemaName) => {
        const schemaKey = `${conn.id}::${db}::schema::${schemaName}`;
        const schemaExpanded = expandedKeys.has(schemaKey);

        const schemaTables = tables.filter((t) => t.schema === schemaName);
        const schemaViews = views.filter((v) => v.name.startsWith(`${schemaName}.`));
        const schemaFunctions = functions.filter((f) => f.name.startsWith(`${schemaName}.`));
        const schemaProcedures = procedures.filter((p) => p.name.startsWith(`${schemaName}.`));
        const schemaTriggers = triggers.filter((t) => t.name.startsWith(`${schemaName}.`));

        const schemaTablesKey = `${schemaKey}::tables`;
        const schemaViewsKey = `${schemaKey}::views`;
        const schemaFunctionsKey = `${schemaKey}::functions`;
        const schemaProceduresKey = `${schemaKey}::procedures`;
        const schemaTriggersKey = `${schemaKey}::triggers`;

        const schemaTablesExpanded = expandedKeys.has(schemaTablesKey);
        const schemaViewsExpanded = expandedKeys.has(schemaViewsKey);
        const schemaFunctionsExpanded = expandedKeys.has(schemaFunctionsKey);
        const schemaProceduresExpanded = expandedKeys.has(schemaProceduresKey);
        const schemaTriggersExpanded = expandedKeys.has(schemaTriggersKey);

        return (
          <div key={schemaKey}>
            {/* Schema node */}
            <ContextMenu>
              <ContextMenuTrigger asChild>
                <button
                  className="flex w-full items-center gap-2 rounded-md py-1.5 pl-14 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
                  onClick={async () => {
                    toggleExpand(schemaKey);
                    if (!schemaExpanded) {
                      // Bug Fix 1: Always load tables for the correct non-default database
                      await loadTables(conn.id, db);
                      await Promise.all([
                        loadViews(conn.id, db),
                        loadFunctions(conn.id, db),
                        loadProcedures(conn.id, db),
                        loadTriggers(conn.id, db),
                      ]);
                    }
                  }}
                >
                  {schemaExpanded
                    ? <ChevronDown className="h-3 w-3 shrink-0 text-muted-foreground" />
                    : <ChevronRight className="h-3 w-3 shrink-0 text-muted-foreground" />
                  }
                  <Layers className="h-3.5 w-3.5 shrink-0 text-indigo-500" />
                  <span className="truncate">{schemaName}</span>
                </button>
              </ContextMenuTrigger>
              <ContextMenuContent className="w-48">
                <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('table', conn.id, db, schemaName, 'postgresql')}>
                  <Table2 className="h-4 w-4" /> {t('sidebar.newTable')}
                </ContextMenuItem>
                <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('view', conn.id, db, schemaName, 'postgresql')}>
                  <Eye className="h-4 w-4" /> {t('sidebar.newView')}
                </ContextMenuItem>
                <ContextMenuItem className="gap-2 py-2" onClick={() => openDialog('function', conn.id, db, schemaName, 'postgresql')}>
                  <FunctionSquare className="h-4 w-4" /> {t('sidebar.newFunction')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenERDiagram(conn.id, db, schemaName)}>
                  <GitBranch className="h-4 w-4" /> {t('sidebar.erDiagram')}
                </ContextMenuItem>
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenQuery(conn.id, db)}>
                  <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleRenameSchema(schemaName)}>
                  <Pencil className="h-4 w-4" /> {t('sidebar.renameSchema')}
                </ContextMenuItem>
                <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropSchema(schemaName)}>
                  <Trash2 className="h-4 w-4" /> {t('sidebar.dropSchema')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => loadSchemas(conn.id, db)}>
                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>

            {schemaExpanded && (
              <>
                {/* Tables under schema */}
                <button
                  className="flex w-full items-center gap-2 rounded-md py-1 pl-20 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                  onClick={() => {
                    toggleExpand(schemaTablesKey);
                    if (!schemaTablesExpanded) loadTables(conn.id, db);
                  }}
                >
                  {tablesLoading
                    ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" />
                    : schemaTablesExpanded
                      ? <ChevronDown className="h-2.5 w-2.5 shrink-0" />
                      : <ChevronRight className="h-2.5 w-2.5 shrink-0" />
                  }
                  <Table2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
                  <span>{t('sidebar.tables')} ({schemaTables.length})</span>
                </button>
                {schemaTablesExpanded && (
                  <VirtualList
                    items={schemaTables}
                    renderItem={(table) => {
                      const qualifiedName = `${schemaName}.${table.name}`;
                      return (
                        <ContextMenu key={`${schemaKey}::table::${table.name}`}>
                          <ContextMenuTrigger asChild>
                            <button
                              className="flex w-full items-center gap-2 rounded-md py-1.5 pl-[6.5rem] pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent"
                              onClick={() => handleViewData(conn.id, db, qualifiedName)}
                              draggable
                              onDragStart={(e) => {
                                e.dataTransfer.setData('application/json', JSON.stringify({
                                  connectionId: conn.id,
                                  database: db,
                                  tableName: qualifiedName,
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
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleViewData(conn.id, db, qualifiedName)}>
                              <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                            </ContextMenuItem>
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleViewStructure(conn.id, db, qualifiedName)}>
                              <Columns className="h-4 w-4" /> {t('sidebar.viewStructure')}
                            </ContextMenuItem>
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleViewChart(conn.id, db, qualifiedName)}>
                              <BarChart3 className="h-4 w-4" /> {t('chart.title')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenQuery(conn.id, db)}>
                              <Code className="h-4 w-4" /> {t('sidebar.openQuery')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleRenameTable(qualifiedName)}>
                              <Pencil className="h-4 w-4" /> {t('sidebar.renameTable')}
                            </ContextMenuItem>
                            <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropTable(qualifiedName)}>
                              <Trash2 className="h-4 w-4" /> {t('sidebar.dropTable')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => {
                              clipboardStore.copyTable({ connectionId: conn.id, database: db, tableName: qualifiedName, dbType: conn.db_type });
                            }}>
                              <Copy className="h-4 w-4" /> {t('tableCopy.copyTable')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => loadTables(conn.id, db)}>
                              <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                            </ContextMenuItem>
                          </ContextMenuContent>
                        </ContextMenu>
                      );
                    }}
                  />
                )}

                {/* Views under schema */}
                <button
                  className="flex w-full items-center gap-2 rounded-md py-1 pl-20 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                  onClick={() => { toggleExpand(schemaViewsKey); if (!schemaViewsExpanded) loadViews(conn.id, db); }}
                >
                  {viewsLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : schemaViewsExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                  <Eye className="h-3.5 w-3.5 shrink-0 text-blue-500" />
                  <span>{t('sidebar.views')} ({schemaViews.length})</span>
                </button>
                {schemaViewsExpanded && (
                  <VirtualList
                    items={schemaViews}
                    renderItem={(view) => {
                      const qualifiedViewName = `${schemaName}.${view.name}`;
                      return (
                        <ContextMenu key={`${schemaKey}::view::${view.name}`}>
                          <ContextMenuTrigger asChild>
                            <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-[6.5rem] pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, qualifiedViewName, 'view')}>
                              <Eye className="h-3.5 w-3.5 shrink-0 text-blue-500" /><span className="truncate">{view.name}</span>
                            </button>
                          </ContextMenuTrigger>
                          <ContextMenuContent className="w-44">
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenObjectDef(conn.id, db, qualifiedViewName, 'view')}>
                              <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropView(qualifiedViewName)}>
                              <Trash2 className="h-4 w-4" /> {t('sidebar.dropView')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => loadViews(conn.id, db)}>
                              <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                            </ContextMenuItem>
                          </ContextMenuContent>
                        </ContextMenu>
                      );
                    }}
                  />
                )}

                {/* Functions under schema */}
                <button
                  className="flex w-full items-center gap-2 rounded-md py-1 pl-20 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                  onClick={() => { toggleExpand(schemaFunctionsKey); if (!schemaFunctionsExpanded) loadFunctions(conn.id, db); }}
                >
                  {functionsLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : schemaFunctionsExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                  <FunctionSquare className="h-3.5 w-3.5 shrink-0 text-purple-500" />
                  <span>{t('sidebar.functions')} ({schemaFunctions.length})</span>
                </button>
                {schemaFunctionsExpanded && (
                  <VirtualList
                    items={schemaFunctions}
                    renderItem={(func) => {
                      const qualifiedFuncName = `${schemaName}.${func.name}`;
                      return (
                        <ContextMenu key={`${schemaKey}::func::${func.name}`}>
                          <ContextMenuTrigger asChild>
                            <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-[6.5rem] pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, qualifiedFuncName, 'function')}>
                              <FunctionSquare className="h-3.5 w-3.5 shrink-0 text-purple-500" /><span className="truncate">{func.name}</span>
                            </button>
                          </ContextMenuTrigger>
                          <ContextMenuContent className="w-44">
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenObjectDef(conn.id, db, qualifiedFuncName, 'function')}>
                              <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropFunction(qualifiedFuncName)}>
                              <Trash2 className="h-4 w-4" /> {t('sidebar.dropFunction')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => loadFunctions(conn.id, db)}>
                              <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                            </ContextMenuItem>
                          </ContextMenuContent>
                        </ContextMenu>
                      );
                    }}
                  />
                )}

                {/* Procedures under schema */}
                <button
                  className="flex w-full items-center gap-2 rounded-md py-1 pl-20 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                  onClick={() => { toggleExpand(schemaProceduresKey); if (!schemaProceduresExpanded) loadProcedures(conn.id, db); }}
                >
                  {proceduresLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : schemaProceduresExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                  <Workflow className="h-3.5 w-3.5 shrink-0 text-orange-500" />
                  <span>{t('sidebar.procedures')} ({schemaProcedures.length})</span>
                </button>
                {schemaProceduresExpanded && (
                  <VirtualList
                    items={schemaProcedures}
                    renderItem={(proc) => {
                      const qualifiedProcName = `${schemaName}.${proc.name}`;
                      return (
                        <ContextMenu key={`${schemaKey}::proc::${proc.name}`}>
                          <ContextMenuTrigger asChild>
                            <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-[6.5rem] pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, qualifiedProcName, 'procedure')}>
                              <Workflow className="h-3.5 w-3.5 shrink-0 text-orange-500" /><span className="truncate">{proc.name}</span>
                            </button>
                          </ContextMenuTrigger>
                          <ContextMenuContent className="w-44">
                            <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenObjectDef(conn.id, db, qualifiedProcName, 'procedure')}>
                              <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropProcedure(qualifiedProcName)}>
                              <Trash2 className="h-4 w-4" /> {t('sidebar.dropProcedure')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => loadProcedures(conn.id, db)}>
                              <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                            </ContextMenuItem>
                          </ContextMenuContent>
                        </ContextMenu>
                      );
                    }}
                  />
                )}

                {/* Triggers under schema */}
                <button
                  className="flex w-full items-center gap-2 rounded-md py-1 pl-20 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
                  onClick={() => { toggleExpand(schemaTriggersKey); if (!schemaTriggersExpanded) loadTriggers(conn.id, db); }}
                >
                  {triggersLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : schemaTriggersExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
                  <Zap className="h-3.5 w-3.5 shrink-0 text-yellow-500" />
                  <span>{t('sidebar.triggers')} ({schemaTriggers.length})</span>
                </button>
                {schemaTriggersExpanded && (
                  <VirtualList
                    items={schemaTriggers}
                    renderItem={(trigger) => {
                      const qualifiedTriggerName = `${schemaName}.${trigger.name}`;
                      return (
                        <ContextMenu key={`${schemaKey}::trigger::${trigger.name}`}>
                          <ContextMenuTrigger asChild>
                            <Tooltip>
                              <TooltipTrigger asChild>
                                <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-[6.5rem] pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent">
                                  <Zap className="h-3.5 w-3.5 shrink-0 text-yellow-500" /><span className="truncate">{trigger.name}</span>
                                </button>
                              </TooltipTrigger>
                              <TooltipContent side="right">{trigger.timing} {trigger.event} ON {trigger.table}</TooltipContent>
                            </Tooltip>
                          </ContextMenuTrigger>
                          <ContextMenuContent className="w-44">
                            <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropTrigger(qualifiedTriggerName)}>
                              <Trash2 className="h-4 w-4" /> {t('sidebar.dropTrigger')}
                            </ContextMenuItem>
                            <ContextMenuSeparator />
                            <ContextMenuItem className="gap-2 py-2" onClick={() => loadTriggers(conn.id, db)}>
                              <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                            </ContextMenuItem>
                          </ContextMenuContent>
                        </ContextMenu>
                      );
                    }}
                  />
                )}
              </>
            )}
          </div>
        );
      })}
    </>
  );
}
