import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem,
  ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
import {
  Table2, ChevronRight, ChevronDown, Eye, Columns, Code,
  FunctionSquare, Workflow, Zap, Copy, BarChart3, Pencil, Trash2, RefreshCw, Loader2,
} from 'lucide-react';
import type { SqlDatabaseTreeProps } from './types';
import { VirtualList } from './VirtualList';
import { useCreateResourceStore } from '@/stores/createResourceStore';
import { notify } from '@/stores/notificationStore';
import { confirm } from '@/stores/confirmStore';
import { useConnectionStore, loadingKey } from '@/stores/connectionStore';
import { getDropTableSQL, getRenameTableSQL, getDropViewSQL, getDropFunctionSQL, getDropProcedureSQL, getDropTriggerSQL } from './sqlHelpers';

export default function SqlDatabaseTree({
  conn, node, db, dbKey, expandedKeys, toggleExpand,
  handleViewData, handleViewStructure, handleViewChart,
  handleOpenObjectDef, handleOpenQuery,
  loadTables, loadViews, loadFunctions, loadProcedures, loadTriggers,
  clipboardStore,
}: SqlDatabaseTreeProps) {
  const { t } = useTranslation();
  const { openDialog } = useCreateResourceStore();
  const loadingKeys = useConnectionStore((s) => s.loadingKeys);
  const tablesLoading = loadingKeys.has(loadingKey.tables(conn.id, db));
  const viewsLoading = loadingKeys.has(loadingKey.views(conn.id, db));
  const functionsLoading = loadingKeys.has(loadingKey.functions(conn.id, db));
  const proceduresLoading = loadingKeys.has(loadingKey.procedures(conn.id, db));
  const triggersLoading = loadingKeys.has(loadingKey.triggers(conn.id, db));

  const handleRenameTable = async (tableName: string) => {
    const newName = window.prompt(t('sidebar.renameTable'), tableName);
    if (!newName || newName === tableName) return;
    try {
      const sql = getRenameTableSQL(tableName, newName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadTables(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropTable = async (tableName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: tableName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropTableSQL(tableName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadTables(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropView = async (viewName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: viewName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropViewSQL(viewName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadViews(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropFunction = async (funcName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: funcName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropFunctionSQL(funcName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadFunctions(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropProcedure = async (procName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: procName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropProcedureSQL(procName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadProcedures(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

  const handleDropTrigger = async (triggerName: string) => {
    const msg = t('sidebar.confirmDropMessage', { name: triggerName });
    const ok = await confirm(t('sidebar.confirmDrop'), msg);
    if (!ok) return;
    try {
      const sql = getDropTriggerSQL(triggerName, conn.db_type);
      await invoke('execute_query', { connectionId: conn.id, database: db, sql });
      notify.success(t('common.success'));
      await loadTriggers(conn.id, db);
    } catch (err: any) {
      notify.error(t('common.error'), String(err));
    }
  };

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

  return (
    <>
      {/* Tables Category */}
      <button
        className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
        onClick={() => {
          toggleExpand(tablesKey);
          if (!tablesExpanded) loadTables(conn.id, db);
        }}
      >
        {tablesLoading
          ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" />
          : tablesExpanded
            ? <ChevronDown className="h-2.5 w-2.5 shrink-0" />
            : <ChevronRight className="h-2.5 w-2.5 shrink-0" />
        }
        <Table2 className="h-3.5 w-3.5 shrink-0 text-emerald-500" />
        <span>{t('sidebar.tables')} ({tables.length})</span>
      </button>
      {tablesExpanded && (
        <VirtualList
          items={tables}
          renderItem={(table) => (
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
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleRenameTable(table.name)}>
                  <Pencil className="h-4 w-4" /> {t('sidebar.renameTable')}
                </ContextMenuItem>
                <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropTable(table.name)}>
                  <Trash2 className="h-4 w-4" /> {t('sidebar.dropTable')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => {
                  clipboardStore.copyTable({ connectionId: conn.id, database: db, tableName: table.name, dbType: conn.db_type });
                }}>
                  <Copy className="h-4 w-4" /> {t('tableCopy.copyTable')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => loadTables(conn.id, db)}>
                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )}
        />
      )}

      {/* Views */}
      <button
        className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
        onClick={() => { toggleExpand(viewsKey); if (!viewsExpanded) loadViews(conn.id, db); }}
      >
        {viewsLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : viewsExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
        <Eye className="h-3.5 w-3.5 shrink-0 text-blue-500" />
        <span>{t('sidebar.views')} ({views.length})</span>
      </button>
      {viewsExpanded && (
        <VirtualList
          items={views}
          renderItem={(view) => (
            <ContextMenu key={`${dbKey}::view::${view.name}`}>
              <ContextMenuTrigger asChild>
                <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, view.name, 'view')}>
                  <Eye className="h-3.5 w-3.5 shrink-0 text-blue-500" /><span className="truncate">{view.name}</span>
                </button>
              </ContextMenuTrigger>
              <ContextMenuContent className="w-44">
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenObjectDef(conn.id, db, view.name, 'view')}>
                  <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropView(view.name)}>
                  <Trash2 className="h-4 w-4" /> {t('sidebar.dropView')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => loadViews(conn.id, db)}>
                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )}
        />
      )}

      {/* Functions */}
      <button
        className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
        onClick={() => { toggleExpand(functionsKey); if (!functionsExpanded) loadFunctions(conn.id, db); }}
      >
        {functionsLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : functionsExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
        <FunctionSquare className="h-3.5 w-3.5 shrink-0 text-purple-500" />
        <span>{t('sidebar.functions')} ({functions.length})</span>
      </button>
      {functionsExpanded && (
        <VirtualList
          items={functions}
          renderItem={(func) => (
            <ContextMenu key={`${dbKey}::func::${func.name}`}>
              <ContextMenuTrigger asChild>
                <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, func.name, 'function')}>
                  <FunctionSquare className="h-3.5 w-3.5 shrink-0 text-purple-500" /><span className="truncate">{func.name}</span>
                </button>
              </ContextMenuTrigger>
              <ContextMenuContent className="w-44">
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenObjectDef(conn.id, db, func.name, 'function')}>
                  <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropFunction(func.name)}>
                  <Trash2 className="h-4 w-4" /> {t('sidebar.dropFunction')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => loadFunctions(conn.id, db)}>
                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )}
        />
      )}

      {/* Procedures */}
      <button
        className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
        onClick={() => { toggleExpand(proceduresKey); if (!proceduresExpanded) loadProcedures(conn.id, db); }}
      >
        {proceduresLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : proceduresExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
        <Workflow className="h-3.5 w-3.5 shrink-0 text-orange-500" />
        <span>{t('sidebar.procedures')} ({procedures.length})</span>
      </button>
      {proceduresExpanded && (
        <VirtualList
          items={procedures}
          renderItem={(proc) => (
            <ContextMenu key={`${dbKey}::proc::${proc.name}`}>
              <ContextMenuTrigger asChild>
                <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent" onClick={() => handleOpenObjectDef(conn.id, db, proc.name, 'procedure')}>
                  <Workflow className="h-3.5 w-3.5 shrink-0 text-orange-500" /><span className="truncate">{proc.name}</span>
                </button>
              </ContextMenuTrigger>
              <ContextMenuContent className="w-44">
                <ContextMenuItem className="gap-2 py-2" onClick={() => handleOpenObjectDef(conn.id, db, proc.name, 'procedure')}>
                  <Eye className="h-4 w-4" /> {t('sidebar.viewData')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropProcedure(proc.name)}>
                  <Trash2 className="h-4 w-4" /> {t('sidebar.dropProcedure')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => loadProcedures(conn.id, db)}>
                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )}
        />
      )}

      {/* Triggers */}
      <button
        className="flex w-full items-center gap-2 rounded-md py-1 pl-14 pr-2.5 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-sidebar-accent"
        onClick={() => { toggleExpand(triggersKey); if (!triggersExpanded) loadTriggers(conn.id, db); }}
      >
        {triggersLoading ? <Loader2 className="h-2.5 w-2.5 shrink-0 animate-spin" /> : triggersExpanded ? <ChevronDown className="h-2.5 w-2.5 shrink-0" /> : <ChevronRight className="h-2.5 w-2.5 shrink-0" />}
        <Zap className="h-3.5 w-3.5 shrink-0 text-yellow-500" />
        <span>{t('sidebar.triggers')} ({triggers.length})</span>
      </button>
      {triggersExpanded && (
        <VirtualList
          items={triggers}
          renderItem={(trigger) => (
            <ContextMenu key={`${dbKey}::trigger::${trigger.name}`}>
              <ContextMenuTrigger asChild>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button className="flex w-full items-center gap-2 rounded-md py-1.5 pl-20 pr-2.5 text-left text-sm transition-colors hover:bg-sidebar-accent">
                      <Zap className="h-3.5 w-3.5 shrink-0 text-yellow-500" /><span className="truncate">{trigger.name}</span>
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right">{trigger.timing} {trigger.event} ON {trigger.table}</TooltipContent>
                </Tooltip>
              </ContextMenuTrigger>
              <ContextMenuContent className="w-44">
                <ContextMenuItem className="gap-2 py-2 text-destructive focus:text-destructive" onClick={() => handleDropTrigger(trigger.name)}>
                  <Trash2 className="h-4 w-4" /> {t('sidebar.dropTrigger')}
                </ContextMenuItem>
                <ContextMenuSeparator />
                <ContextMenuItem className="gap-2 py-2" onClick={() => loadTriggers(conn.id, db)}>
                  <RefreshCw className="h-4 w-4" /> {t('sidebar.refresh')}
                </ContextMenuItem>
              </ContextMenuContent>
            </ContextMenu>
          )}
        />
      )}
    </>
  );
}
