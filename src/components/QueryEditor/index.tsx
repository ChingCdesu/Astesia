import { useState, useRef, useCallback, useEffect, useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { ScrollArea } from '@/components/ui/scroll-area';
import Editor, { OnMount, BeforeMount } from '@monaco-editor/react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { StatementResult, TableInfo, ColumnInfo } from '@/types/database';
import {
  Play, Eraser, Download, Loader2, FolderOpen, Save, RefreshCw, BarChart3, Table2, Lightbulb, Copy, Check, AlertCircle, CheckCircle2,
} from 'lucide-react';
import type { editor } from 'monaco-editor';
import { cn } from '@/lib/utils';
import {
  flushTabContent,
  stageTabContent,
} from '@/stores/tabStore';
import { useThemeStore } from '@/stores/themeStore';
import { configureMonacoForDialect, SqlDialect, registerDatabaseCompletions, clearDatabaseCompletions } from '@/lib/monacoSetup';
import { open, save } from '@tauri-apps/plugin-dialog';
import {
  BarChart, Bar, LineChart, Line, AreaChart, Area,
  ScatterChart, Scatter, PieChart, Pie, Cell,
  XAxis, YAxis, CartesianGrid, Tooltip, Legend,
  ResponsiveContainer,
} from 'recharts';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { readTextFile, writeTextFile } from '@tauri-apps/plugin-fs';
import { splitSqlStatements, isTransactionControl } from '@/lib/sqlSplit';
import ExportDialog from '@/components/ExportDialog';
import { notify } from '@/stores/notificationStore';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';

interface Props {
  connectionId: string;
  database: string;
  tabKey: string;
  dbType?: string;
  initialContent?: string;
}

// Dialects that support an EXPLAIN-prefix that returns a result set the way
// SELECT does. SQL Server uses different mechanisms (SHOWPLAN) so it gets a
// dedicated branch in `buildExplainSql`.
const EXPLAINABLE_DIALECTS: ReadonlyArray<string> = ['mysql', 'postgresql', 'sqlite', 'sqlserver'];

function buildExplainSql(sql: string, dbType: string | undefined): string {
  const trimmed = sql.trim().replace(/;\s*$/, '');
  switch (dbType) {
    case 'sqlite':
      return `EXPLAIN QUERY PLAN ${trimmed}`;
    case 'sqlserver':
      // SHOWPLAN_ALL returns plan rows for a subsequent batch, then must be
      // turned off again. Wrap in semicolons so the splitter sees three
      // distinct statements that all run on the same connection.
      return `SET SHOWPLAN_ALL ON; ${trimmed}; SET SHOWPLAN_ALL OFF`;
    default:
      return `EXPLAIN ${trimmed}`;
  }
}

export default function QueryEditor({ connectionId, database, tabKey, dbType, initialContent }: Props) {
  const { t } = useTranslation();
  const [results, setResults] = useState<StatementResult[]>([]);
  const [activeIdx, setActiveIdx] = useState(0);
  const [loading, setLoading] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);
  const [showChart, setShowChart] = useState(false);
  const [editorHeight, setEditorHeight] = useState(250);
  const [isResizingEditor, setIsResizingEditor] = useState(false);
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const monacoInstanceRef = useRef<typeof import('monaco-editor') | null>(null);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const editorResizeRef = useRef({ startY: 0, startHeight: 0 });
  const resolvedTheme = useThemeStore((s) => s.resolvedTheme);

  useEffect(() => {
    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
      const currentContent = editorRef.current?.getValue();
      if (currentContent !== undefined) {
        stageTabContent(tabKey, currentContent);
      }
      flushTabContent(tabKey);
    };
  }, [tabKey]);

  // Fetch database metadata and register Monaco autocompletions
  useEffect(() => {
    let cancelled = false;
    const loadCompletions = async () => {
      try {
        const tables = await invoke<TableInfo[]>('get_tables', { connectionId, database });
        if (cancelled) return;

        const tablesToFetch = tables.slice(0, 50);
        const tableData = await Promise.all(
          tablesToFetch.map(async (t) => {
            const tableName =
              t.schema && dbType === 'postgresql' ? `${t.schema}.${t.name}` : t.name;
            try {
              const cols = await invoke<ColumnInfo[]>('get_columns', {
                connectionId,
                database,
                table: tableName,
              });
              return {
                name: t.name,
                schema: t.schema || undefined,
                columns: cols.map((c) => ({ name: c.name, type: c.data_type })),
              };
            } catch {
              return { name: t.name, schema: t.schema || undefined, columns: [] };
            }
          }),
        );

        if (cancelled || !monacoInstanceRef.current) return;
        registerDatabaseCompletions(monacoInstanceRef.current, { tables: tableData }, dbType || '');
      } catch (e) {
        console.error('Failed to load database completions:', e);
      }
    };

    loadCompletions();
    return () => {
      cancelled = true;
      clearDatabaseCompletions();
    };
  }, [connectionId, database, dbType]);

  const handleEditorResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizingEditor(true);
    editorResizeRef.current = { startY: e.clientY, startHeight: editorHeight };
  }, [editorHeight]);

  useEffect(() => {
    if (!isResizingEditor) return;
    const handleMouseMove = (e: MouseEvent) => {
      const delta = e.clientY - editorResizeRef.current.startY;
      setEditorHeight(Math.max(120, Math.min(500, editorResizeRef.current.startHeight + delta)));
    };
    const handleMouseUp = () => setIsResizingEditor(false);
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    document.body.style.cursor = 'row-resize';
    document.body.style.userSelect = 'none';
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
  }, [isResizingEditor]);

  const handleContentChange = useCallback(
    (value: string | undefined) => {
      stageTabContent(tabKey, value ?? '');
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
      debounceTimerRef.current = setTimeout(() => {
        flushTabContent(tabKey);
      }, 500);
    },
    [tabKey]
  );

  const handleBeforeMount: BeforeMount = (monaco) => {
    monacoInstanceRef.current = monaco;
    if (dbType) {
      configureMonacoForDialect(monaco, dbType as SqlDialect);
    }
  };

  // Returns the SQL the user wants to run. If a non-empty selection exists
  // we honor that; otherwise the entire buffer is used.
  const collectSqlToRun = useCallback((): string => {
    const ed = editorRef.current;
    if (!ed) return '';
    const selection = ed.getSelection();
    if (selection && !selection.isEmpty()) {
      return ed.getModel()?.getValueInRange(selection) || '';
    }
    return ed.getValue();
  }, []);

  // Returns the SQL of the statement at the cursor, used by the EXPLAIN
  // button so the user can place the caret in any of several statements and
  // have just that one explained.
  const collectStatementAtCursor = useCallback((): string => {
    const sql = collectSqlToRun();
    const ed = editorRef.current;
    if (!ed) return sql;
    const selection = ed.getSelection();
    if (selection && !selection.isEmpty()) return sql;
    const statements = splitSqlStatements(sql);
    if (statements.length <= 1) return statements[0]?.sql ?? sql;
    const cursorLine = ed.getPosition()?.lineNumber ?? 1;
    let chosen = statements[0];
    for (const s of statements) {
      if (s.startLine <= cursorLine) chosen = s;
      else break;
    }
    return chosen.sql;
  }, [collectSqlToRun]);

  const runStatements = useCallback(async (sql: string) => {
    if (!sql.trim()) return;
    const statements = splitSqlStatements(sql).map((s) => s.sql).filter((s) => s.trim());
    if (statements.length === 0) return;

    setLoading(true);
    setResults([]);
    setActiveIdx(0);
    setShowChart(false);
    try {
      const res = await invoke<StatementResult[]>('execute_statements', {
        connectionId,
        database,
        statements,
      });
      setResults(res);
      // Jump to first failed statement, otherwise first one
      const firstFailIdx = res.findIndex((r) => !r.success);
      setActiveIdx(firstFailIdx >= 0 ? firstFailIdx : 0);
    } catch (e) {
      // Backend returned an Err — surface as a single synthetic failure entry
      const message = typeof e === 'string' ? e : (e instanceof Error ? e.message : String(e));
      setResults([
        {
          sql,
          success: false,
          error: message,
          columns: [],
          rows: [],
          affected_rows: 0,
          execution_time_ms: 0,
        },
      ]);
      setActiveIdx(0);
    } finally {
      setLoading(false);
    }
  }, [connectionId, database]);

  const handleExecute = useCallback(() => {
    runStatements(collectSqlToRun());
  }, [collectSqlToRun, runStatements]);

  const handleExplain = useCallback(() => {
    if (!dbType || !EXPLAINABLE_DIALECTS.includes(dbType)) return;
    const stmt = collectStatementAtCursor().trim();
    if (!stmt) return;
    runStatements(buildExplainSql(stmt, dbType));
  }, [collectStatementAtCursor, runStatements, dbType]);

  const handleEditorMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    editor.addAction({
      id: 'execute-query',
      label: 'Execute Query',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.Enter],
      run: () => handleExecute(),
    });
    editor.addAction({
      id: 'open-file',
      label: 'Open SQL File',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyO],
      run: () => handleOpenFile(),
    });
    editor.addAction({
      id: 'save-file',
      label: 'Save SQL File',
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
      run: () => handleSaveFile(),
    });
  };

  const handleClear = () => {
    editorRef.current?.setValue('');
    setResults([]);
  };

  const activeResult = results[activeIdx];

  const handleExport = () => {
    if (!activeResult || activeResult.rows.length === 0) return;
    setExportOpen(true);
  };

  const handleOpenFile = useCallback(async () => {
    const path = await open({ filters: [{ name: 'SQL Files', extensions: ['sql'] }] });
    if (path) {
      const content = await readTextFile(path);
      editorRef.current?.setValue(content);
    }
  }, []);

  const handleSaveFile = useCallback(async () => {
    const content = editorRef.current?.getValue() ?? '';
    const path = await save({ defaultPath: 'query.sql', filters: [{ name: 'SQL Files', extensions: ['sql'] }] });
    if (path) {
      await writeTextFile(path, content);
    }
  }, []);

  const showExplain = dbType ? EXPLAINABLE_DIALECTS.includes(dbType) : false;
  const multipleResults = results.length > 1;
  const hasTransaction = useMemo(
    () => results.some((r) => isTransactionControl(r.sql)),
    [results]
  );

  return (
    <div className="flex h-full flex-col">
      {/* Toolbar */}
      <div className="flex shrink-0 items-center gap-2 border-b bg-muted/30 px-4 py-2">
        <Button size="sm" onClick={handleExecute} disabled={loading}>
          {loading
            ? <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />
            : <Play className="mr-1.5 h-3.5 w-3.5" />
          }
          {t('query.execute')}
        </Button>
        {showExplain && (
          <Button variant="ghost" size="sm" onClick={handleExplain} disabled={loading} title={t('query.explain')}>
            <Lightbulb className="mr-1.5 h-3.5 w-3.5" />
            {t('query.explain')}
          </Button>
        )}
        <Button variant="ghost" size="sm" onClick={handleClear}>
          <Eraser className="mr-1.5 h-3.5 w-3.5" />
          {t('query.clear')}
        </Button>
        <Button variant="ghost" size="sm" onClick={handleExport} disabled={!activeResult || activeResult.rows.length === 0}>
          <Download className="mr-1.5 h-3.5 w-3.5" />
          {t('query.export')}
        </Button>
        <Button variant="ghost" size="sm" onClick={handleOpenFile}>
          <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
          {t('query.open')}
        </Button>
        <Button variant="ghost" size="sm" onClick={handleSaveFile}>
          <Save className="mr-1.5 h-3.5 w-3.5" />
          {t('query.save')}
        </Button>
        <div className="ml-auto">
          <Badge variant="outline" className="font-mono text-[11px]">{database}</Badge>
        </div>
      </div>

      {/* Editor */}
      <div className="shrink-0" style={{ height: editorHeight, minHeight: 120, maxHeight: 500 }}>
        <Editor
          height="100%"
          defaultLanguage="sql"
          defaultValue={initialContent ?? ''}
          theme={resolvedTheme === 'dark' ? 'vs-dark' : 'vs'}
          beforeMount={handleBeforeMount}
          onMount={handleEditorMount}
          onChange={handleContentChange}
          options={{
            minimap: { enabled: false },
            fontSize: 13,
            lineNumbers: 'on',
            scrollBeyondLastLine: false,
            wordWrap: 'on',
            automaticLayout: true,
            tabSize: 2,
            padding: { top: 8, bottom: 8 },
            renderLineHighlight: 'line',
          }}
        />
      </div>

      {/* Editor resize handle */}
      <div
        className={cn(
          "h-1 shrink-0 cursor-row-resize transition-colors hover:bg-primary/20",
          isResizingEditor && "bg-primary/30"
        )}
        onMouseDown={handleEditorResizeStart}
      />

      {/* Statement tabs (when multiple) */}
      {multipleResults && (
        <div className="flex shrink-0 items-center gap-1 overflow-x-auto border-b bg-muted/40 px-2 py-1">
          <span className="mr-1 shrink-0 text-[10px] text-muted-foreground">
            {t('query.statementCount', { count: results.length })}
          </span>
          {hasTransaction && (
            <span
              className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-600"
              title={t('query.transactionDetected')}
            >
              TX
            </span>
          )}
          <div className="mx-1 h-3 w-px bg-border" />
          {results.map((r, i) => (
            <button
              key={i}
              onClick={() => { setActiveIdx(i); setShowChart(false); }}
              className={cn(
                "flex shrink-0 items-center gap-1 rounded px-2 py-0.5 text-[11px] font-mono transition-colors",
                i === activeIdx ? "bg-background shadow-sm" : "hover:bg-muted",
                !r.success && "text-red-600"
              )}
              title={r.sql.slice(0, 200)}
            >
              {r.success
                ? <CheckCircle2 className="h-3 w-3 text-emerald-600" />
                : <AlertCircle className="h-3 w-3 text-red-600" />
              }
              #{i + 1}
              <span className="text-muted-foreground">{r.execution_time_ms}ms</span>
            </button>
          ))}
        </div>
      )}

      {/* Result toolbar (per active statement) */}
      {activeResult && (
        <div className="flex shrink-0 items-center gap-2 border-b bg-muted/20 px-4 py-1">
          <span className="text-xs font-medium text-muted-foreground">
            {multipleResults ? `#${activeIdx + 1} ` : ''}{t('query.result')}
          </span>
          <div className="mx-1 h-3 w-px bg-border" />
          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={handleExecute} disabled={loading}>
            <RefreshCw className={cn("mr-1 h-3 w-3", loading && "animate-spin")} />
            {t('query.refresh')}
          </Button>
          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={handleExport} disabled={!activeResult.rows.length}>
            <Download className="mr-1 h-3 w-3" />
            {t('query.export')}
          </Button>
          {activeResult.success && activeResult.columns.length > 0 && (
            <Button
              variant="ghost"
              size="sm"
              className="h-6 px-2 text-xs"
              onClick={() => setShowChart((s) => !s)}
            >
              {showChart ? (
                <>
                  <Table2 className="mr-1 h-3 w-3" />
                  {t('query.table')}
                </>
              ) : (
                <>
                  <BarChart3 className="mr-1 h-3 w-3" />
                  {t('query.chart')}
                </>
              )}
            </Button>
          )}
          <span className="ml-auto text-[10px] text-muted-foreground">
            {activeResult.success
              ? `${activeResult.rows.length} ${t('query.rows')} | ${activeResult.execution_time_ms}ms`
              : `${t('query.failed')} | ${activeResult.execution_time_ms}ms`}
          </span>
        </div>
      )}

      {/* Results */}
      <div className="flex-1 overflow-hidden">
        {loading ? (
          <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
            <span>{t('query.executing')}</span>
          </div>
        ) : !activeResult ? (
          <div className="flex h-full flex-col items-center justify-center gap-1 text-muted-foreground">
            <span className="text-sm">{t('query.placeholderTip')}</span>
          </div>
        ) : !activeResult.success ? (
          <ErrorPanel sql={activeResult.sql} error={activeResult.error || ''} />
        ) : activeResult.columns.length === 0 ? (
          <div className="px-4 py-3 text-sm text-emerald-600">
            {t('query.affected')}: {activeResult.affected_rows} {t('query.rows')}
          </div>
        ) : showChart ? (
          <QueryChartView result={activeResult} />
        ) : (
          <ResultTable result={activeResult} />
        )}
      </div>

      {activeResult && activeResult.columns.length > 0 && (
        <ExportDialog
          open={exportOpen}
          onClose={() => setExportOpen(false)}
          source={{
            kind: 'static',
            columns: activeResult.columns,
            rows: activeResult.rows,
            dbType,
            defaultName: 'query_result',
          }}
        />
      )}
    </div>
  );
}

/* Result table — supports cell-range drag-selection, row selection via the #
 * column, Ctrl/Cmd+C copy (TSV), Ctrl/Cmd+A select-all, and a context menu
 * with "Copy with headers". */
function ResultTable({ result }: { result: StatementResult }) {
  const { t } = useTranslation();
  const containerRef = useRef<HTMLDivElement>(null);

  const [selectionStart, setSelectionStart] = useState<{ row: number; col: number } | null>(null);
  const [selectionEnd, setSelectionEnd] = useState<{ row: number; col: number } | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [lastSelectedRow, setLastSelectedRow] = useState<number | null>(null);

  // Reset selection when the underlying result changes
  useEffect(() => {
    setSelectionStart(null);
    setSelectionEnd(null);
    setSelectedRows(new Set());
    setLastSelectedRow(null);
  }, [result]);

  useEffect(() => {
    const onUp = () => setIsDragging(false);
    document.addEventListener('mouseup', onUp);
    return () => document.removeEventListener('mouseup', onUp);
  }, []);

  const isCellInSelection = useCallback((r: number, c: number) => {
    if (!selectionStart || !selectionEnd) return false;
    const minR = Math.min(selectionStart.row, selectionEnd.row);
    const maxR = Math.max(selectionStart.row, selectionEnd.row);
    const minC = Math.min(selectionStart.col, selectionEnd.col);
    const maxC = Math.max(selectionStart.col, selectionEnd.col);
    return r >= minR && r <= maxR && c >= minC && c <= maxC;
  }, [selectionStart, selectionEnd]);

  const formatCell = (v: unknown): string => {
    if (v === null || v === undefined) return '';
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  };

  const toTsv = (rows: unknown[][]): string =>
    rows
      .map((row) =>
        row
          .map((v) => {
            const s = formatCell(v);
            // TSV-quote when a value contains a delimiter, newline, or quote
            return s.includes('\t') || s.includes('\n') || s.includes('\r') || s.includes('"')
              ? `"${s.replace(/"/g, '""')}"`
              : s;
          })
          .join('\t')
      )
      .join('\n');

  const copyToClipboard = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      notify.success(t('query.copied'));
    } catch {
      /* clipboard write may fail in some environments — ignore */
    }
  };

  const copyCellRange = useCallback(async (withHeaders: boolean) => {
    if (!selectionStart || !selectionEnd) return;
    const minR = Math.min(selectionStart.row, selectionEnd.row);
    const maxR = Math.max(selectionStart.row, selectionEnd.row);
    const minC = Math.min(selectionStart.col, selectionEnd.col);
    const maxC = Math.max(selectionStart.col, selectionEnd.col);
    const out: unknown[][] = [];
    if (withHeaders) out.push(result.columns.slice(minC, maxC + 1).map((c) => c.name));
    for (let r = minR; r <= maxR; r++) {
      const row = result.rows[r];
      if (row) out.push(row.slice(minC, maxC + 1));
    }
    await copyToClipboard(toTsv(out));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectionStart, selectionEnd, result]);

  const copySelectedRows = useCallback(async (withHeaders: boolean) => {
    if (selectedRows.size === 0) return;
    const sorted = Array.from(selectedRows).sort((a, b) => a - b);
    const out: unknown[][] = [];
    if (withHeaders) out.push(result.columns.map((c) => c.name));
    for (const r of sorted) {
      const row = result.rows[r];
      if (row) out.push(row);
    }
    await copyToClipboard(toTsv(out));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedRows, result]);

  const handleCopy = useCallback(
    async (withHeaders: boolean) => {
      if (selectionStart && selectionEnd) await copyCellRange(withHeaders);
      else if (selectedRows.size > 0) await copySelectedRows(withHeaders);
    },
    [selectionStart, selectionEnd, selectedRows, copyCellRange, copySelectedRows]
  );

  const selectAll = useCallback(() => {
    setSelectedRows(new Set(result.rows.map((_, i) => i)));
    setSelectionStart(null);
    setSelectionEnd(null);
  }, [result.rows]);

  const clearSelection = () => {
    setSelectedRows(new Set());
    setSelectionStart(null);
    setSelectionEnd(null);
  };

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      if ((e.ctrlKey || e.metaKey) && e.key === 'c') {
        if ((selectionStart && selectionEnd) || selectedRows.size > 0) {
          e.preventDefault();
          void handleCopy(false);
        }
      } else if ((e.ctrlKey || e.metaKey) && e.key === 'a') {
        e.preventDefault();
        selectAll();
      } else if (e.key === 'Escape') {
        clearSelection();
      }
    };
    el.addEventListener('keydown', onKey);
    return () => el.removeEventListener('keydown', onKey);
  }, [handleCopy, selectAll, selectionStart, selectionEnd, selectedRows]);

  const handleCellMouseDown = (r: number, c: number, e: React.MouseEvent) => {
    if (e.button !== 0) return; // let right-click open the context menu without resetting
    setSelectedRows(new Set());
    if (e.shiftKey && selectionStart) {
      setSelectionEnd({ row: r, col: c });
    } else {
      setSelectionStart({ row: r, col: c });
      setSelectionEnd({ row: r, col: c });
    }
    setIsDragging(true);
    containerRef.current?.focus();
  };

  const handleCellMouseEnter = (r: number, c: number) => {
    if (isDragging) setSelectionEnd({ row: r, col: c });
  };

  const handleRowNumMouseDown = (ri: number, e: React.MouseEvent) => {
    if (e.button !== 0) return;
    setSelectionStart(null);
    setSelectionEnd(null);
    if (e.shiftKey && lastSelectedRow !== null) {
      const lo = Math.min(lastSelectedRow, ri);
      const hi = Math.max(lastSelectedRow, ri);
      const next = new Set(selectedRows);
      for (let i = lo; i <= hi; i++) next.add(i);
      setSelectedRows(next);
    } else if (e.ctrlKey || e.metaKey) {
      const next = new Set(selectedRows);
      if (next.has(ri)) next.delete(ri);
      else next.add(ri);
      setSelectedRows(next);
      setLastSelectedRow(ri);
    } else {
      setSelectedRows(new Set([ri]));
      setLastSelectedRow(ri);
    }
    containerRef.current?.focus();
  };

  const hasSelection = (selectionStart !== null && selectionEnd !== null) || selectedRows.size > 0;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div ref={containerRef} tabIndex={-1} className="h-full outline-none">
          <ScrollArea className="h-full">
            <table
              className="text-sm select-none"
              style={{ tableLayout: 'fixed', width: 'max-content', minWidth: '100%' }}
            >
              <thead className="sticky top-0 z-10">
                <tr className="border-b bg-muted/60">
                  <th className="w-12 px-3 py-2 text-center text-xs font-medium text-muted-foreground">#</th>
                  {result.columns.map((col) => (
                    <th
                      key={col.name}
                      style={{ width: 160, minWidth: 80 }}
                      className="whitespace-nowrap border-l px-4 py-2 text-left text-xs font-medium"
                    >
                      {col.name}
                      <span className="ml-2 font-normal text-muted-foreground">{col.data_type}</span>
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {result.rows.map((row, ri) => {
                  const rowSelected = selectedRows.has(ri);
                  return (
                    <tr key={ri} className={cn('border-b transition-colors', rowSelected ? 'bg-primary/10' : 'hover:bg-muted/30')}>
                      <td
                        className={cn(
                          'px-3 py-1.5 text-center text-xs cursor-pointer',
                          rowSelected ? 'bg-primary/20 text-primary font-medium' : 'text-muted-foreground'
                        )}
                        onMouseDown={(e) => handleRowNumMouseDown(ri, e)}
                      >
                        {ri + 1}
                      </td>
                      {row.map((cell, ci) => {
                        const inRange = isCellInSelection(ri, ci);
                        return (
                          <td
                            key={ci}
                            className={cn(
                              'border-l px-4 py-1.5 font-mono text-xs overflow-hidden cursor-cell',
                              cell === null && 'italic text-muted-foreground/50',
                              inRange && 'bg-primary/20'
                            )}
                            style={{ maxWidth: 300 }}
                            onMouseDown={(e) => handleCellMouseDown(ri, ci, e)}
                            onMouseEnter={() => handleCellMouseEnter(ri, ci)}
                          >
                            <span className="truncate block">
                              {cell === null ? 'NULL' : typeof cell === 'object' ? JSON.stringify(cell) : String(cell)}
                            </span>
                          </td>
                        );
                      })}
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </ScrollArea>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem disabled={!hasSelection} onClick={() => void handleCopy(false)}>
          {t('query.copy')}
        </ContextMenuItem>
        <ContextMenuItem disabled={!hasSelection} onClick={() => void handleCopy(true)}>
          {t('query.copyWithHeaders')}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onClick={selectAll}>{t('query.selectAllRows')}</ContextMenuItem>
        <ContextMenuItem disabled={!hasSelection} onClick={clearSelection}>
          {t('query.clearSelection')}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/* Error panel with copy button */
function ErrorPanel({ sql, error }: { sql: string; error: string }) {
  const { t } = useTranslation();
  const [copied, setCopied] = useState(false);

  const copy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(error);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* no-op */
    }
  }, [error]);

  return (
    <ScrollArea className="h-full">
      <div className="px-4 py-3">
        <div className="mb-2 flex items-center gap-2">
          <AlertCircle className="h-4 w-4 text-red-600" />
          <span className="text-sm font-medium text-red-600">{t('query.queryError')}</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-6 px-2 text-xs"
            onClick={copy}
            title={t('query.copy')}
          >
            {copied
              ? <><Check className="mr-1 h-3 w-3" />{t('query.copied')}</>
              : <><Copy className="mr-1 h-3 w-3" />{t('query.copy')}</>
            }
          </Button>
        </div>
        <pre className="select-text whitespace-pre-wrap rounded bg-red-50/50 px-3 py-2 font-mono text-xs text-red-600 dark:bg-red-950/30 dark:text-red-300">
          {error}
        </pre>
        {sql.trim() && (
          <pre className="mt-2 select-text whitespace-pre-wrap rounded bg-muted/40 px-3 py-2 font-mono text-[11px] text-muted-foreground">
            {sql}
          </pre>
        )}
      </div>
    </ScrollArea>
  );
}

/* Inline chart view for query results — mirrors DataChartView's chart-type
 * selector, multi-Y selection, and aggregation-by-X-value behavior. */
type ChartType = 'bar' | 'line' | 'area' | 'scatter' | 'pie';

const CHART_COLORS = [
  'hsl(210,80%,55%)', 'hsl(150,70%,45%)', 'hsl(350,75%,55%)', 'hsl(40,85%,55%)',
  'hsl(270,65%,55%)', 'hsl(180,60%,45%)', 'hsl(320,70%,55%)', 'hsl(20,80%,55%)',
];

function QueryChartView({ result }: { result: StatementResult }) {
  const { t } = useTranslation();
  const [chartType, setChartType] = useState<ChartType>('bar');
  const [xAxis, setXAxis] = useState<string>('');
  const [yAxes, setYAxes] = useState<string[]>([]);

  const columns = result.columns;

  // Detect numeric vs string columns by sampling the first ~10 rows. Numeric
  // columns become Y-axis candidates; string columns are preferred for X and
  // trigger group-by aggregation when used as the X axis.
  const { numericColumns, stringColumns } = useMemo(() => {
    if (result.rows.length === 0) {
      return { numericColumns: [] as string[], stringColumns: [] as string[] };
    }
    const numeric: string[] = [];
    const str: string[] = [];
    const sampleSize = Math.min(result.rows.length, 10);
    columns.forEach((col, colIndex) => {
      let numCount = 0;
      for (let i = 0; i < sampleSize; i++) {
        const val = result.rows[i][colIndex];
        if (val === null || val === undefined) continue;
        if (typeof val === 'number' || (typeof val === 'string' && val !== '' && !isNaN(Number(val)))) {
          numCount++;
        }
      }
      if (numCount > sampleSize / 2) numeric.push(col.name);
      else str.push(col.name);
    });
    return { numericColumns: numeric, stringColumns: str };
  }, [columns, result.rows]);

  useEffect(() => {
    if (!xAxis) {
      if (stringColumns.length > 0) setXAxis(stringColumns[0]);
      else if (columns.length > 0) setXAxis(columns[0].name);
    }
    if (yAxes.length === 0 && numericColumns.length > 0) {
      setYAxes([numericColumns[0]]);
    }
  }, [columns, numericColumns, stringColumns, xAxis, yAxes.length]);

  // When the X axis is a string column we group rows by X value and SUM
  // numeric Y columns; we also expose `_count` for "rows per X". Otherwise we
  // emit raw rows so the chart shows every data point.
  const chartData = useMemo(() => {
    if (!xAxis) return [];
    const xColIndex = columns.findIndex((c) => c.name === xAxis);
    if (xColIndex < 0) return [];

    if (stringColumns.includes(xAxis)) {
      const grouped = new Map<string, { count: number; sums: Record<string, number> }>();
      result.rows.forEach((row) => {
        const xVal = String(row[xColIndex] ?? '');
        if (!grouped.has(xVal)) grouped.set(xVal, { count: 0, sums: {} });
        const entry = grouped.get(xVal)!;
        entry.count++;
        columns.forEach((col, i) => {
          if (col.name === xAxis) return;
          const val = row[i];
          const num = typeof val === 'number'
            ? val
            : (typeof val === 'string' && val !== '' && !isNaN(Number(val)) ? Number(val) : 0);
          entry.sums[col.name] = (entry.sums[col.name] || 0) + num;
        });
      });
      return Array.from(grouped.entries()).map(([xVal, entry]) => {
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const obj: Record<string, any> = { [xAxis]: xVal, _count: entry.count };
        Object.entries(entry.sums).forEach(([col, sum]) => {
          obj[col] = Math.round(sum * 100) / 100;
        });
        return obj;
      });
    }

    return result.rows.map((row) => {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const obj: Record<string, any> = {};
      columns.forEach((col, i) => {
        const val = row[i];
        obj[col.name] = typeof val === 'string' && val !== '' && !isNaN(Number(val))
          ? Number(val)
          : val;
      });
      return obj;
    });
  }, [result.rows, columns, xAxis, stringColumns]);

  const allColumns = useMemo(() => columns.map((c) => c.name), [columns]);
  const aggregating = stringColumns.includes(xAxis);

  const toggleY = (col: string) => {
    setYAxes((prev) => prev.includes(col) ? prev.filter((c) => c !== col) : [...prev, col]);
  };

  const tooltipStyle = {
    background: 'var(--popover)',
    border: '1px solid var(--border)',
    borderRadius: '6px',
    color: 'var(--popover-foreground)',
    fontSize: 12,
  };

  const chartTypeOptions: { value: ChartType; label: string }[] = [
    { value: 'bar', label: t('chart.bar') },
    { value: 'line', label: t('chart.line') },
    { value: 'area', label: t('chart.area') },
    { value: 'scatter', label: t('chart.scatter') },
    { value: 'pie', label: t('chart.pie') },
  ];

  const renderChart = () => {
    if (chartData.length === 0 || !xAxis || yAxes.length === 0) {
      return (
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          {numericColumns.length === 0 && !aggregating
            ? t('chart.noNumericColumns')
            : t('chart.noData')}
        </div>
      );
    }

    switch (chartType) {
      case 'bar':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} tick={{ fontSize: 11 }} />
              <YAxis tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
              {yAxes.map((col, i) => (
                <Bar key={col} dataKey={col} fill={CHART_COLORS[i % CHART_COLORS.length]} />
              ))}
            </BarChart>
          </ResponsiveContainer>
        );
      case 'line':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} tick={{ fontSize: 11 }} />
              <YAxis tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
              {yAxes.map((col, i) => (
                <Line key={col} type="monotone" dataKey={col} stroke={CHART_COLORS[i % CHART_COLORS.length]} strokeWidth={2} dot={{ r: 3 }} />
              ))}
            </LineChart>
          </ResponsiveContainer>
        );
      case 'area':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} tick={{ fontSize: 11 }} />
              <YAxis tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
              {yAxes.map((col, i) => (
                <Area key={col} type="monotone" dataKey={col} fill={CHART_COLORS[i % CHART_COLORS.length]} stroke={CHART_COLORS[i % CHART_COLORS.length]} fillOpacity={0.3} />
              ))}
            </AreaChart>
          </ResponsiveContainer>
        );
      case 'scatter':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <ScatterChart>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} name={xAxis} tick={{ fontSize: 11 }} />
              <YAxis dataKey={yAxes[0]} name={yAxes[0]} tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} cursor={{ strokeDasharray: '3 3' }} />
              <Legend />
              <Scatter name={`${xAxis} / ${yAxes[0]}`} data={chartData} fill={CHART_COLORS[0]} />
            </ScatterChart>
          </ResponsiveContainer>
        );
      case 'pie':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <PieChart>
              <Pie
                data={chartData}
                dataKey={yAxes[0]}
                nameKey={xAxis}
                cx="50%"
                cy="50%"
                outerRadius={120}
                // eslint-disable-next-line @typescript-eslint/no-explicit-any
                label={({ name, percent }: any) => `${name ?? ''}: ${((percent ?? 0) * 100).toFixed(0)}%`}
              >
                {chartData.map((_, i) => (
                  <Cell key={i} fill={CHART_COLORS[i % CHART_COLORS.length]} />
                ))}
              </Pie>
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
            </PieChart>
          </ResponsiveContainer>
        );
      default:
        return null;
    }
  };

  return (
    <div className="flex h-full">
      {/* Config sidebar */}
      <div className="w-48 shrink-0 overflow-y-auto border-r bg-muted/20 p-3">
        <div className="mb-3">
          <label className="mb-1 block text-[10px] font-medium text-muted-foreground">
            {t('chart.chartType')}
          </label>
          <Select value={chartType} onValueChange={(v) => setChartType(v as ChartType)}>
            <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
            <SelectContent>
              {chartTypeOptions.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="mb-3">
          <label className="mb-1 block text-[10px] font-medium text-muted-foreground">
            {t('chart.xAxis')}
          </label>
          <Select value={xAxis} onValueChange={setXAxis}>
            <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
            <SelectContent>
              {allColumns.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
        <div>
          <label className="mb-1 block text-[10px] font-medium text-muted-foreground">
            {t('chart.yAxis')}
          </label>
          <div className="flex flex-col gap-0.5">
            {/* `_count` only makes sense when X is a grouping (string) column */}
            {aggregating && (
              <label className="flex cursor-pointer items-center gap-1.5 rounded bg-muted/30 px-1.5 py-1 text-xs hover:bg-muted/50">
                <input
                  type="checkbox"
                  checked={yAxes.includes('_count')}
                  onChange={() => toggleY('_count')}
                  className="h-3 w-3 rounded"
                />
                <span className="truncate italic">_count (聚合计数)</span>
              </label>
            )}
            {numericColumns.length > 0 ? (
              numericColumns.map((col) => (
                <label
                  key={col}
                  className="flex cursor-pointer items-center gap-1.5 rounded px-1.5 py-1 text-xs hover:bg-muted/50"
                >
                  <input
                    type="checkbox"
                    checked={yAxes.includes(col)}
                    onChange={() => toggleY(col)}
                    className="h-3 w-3 rounded"
                  />
                  <span className="truncate">{col}</span>
                </label>
              ))
            ) : (
              !aggregating && (
                <p className="text-[10px] italic text-muted-foreground">
                  {t('chart.noNumericColumns')}
                </p>
              )
            )}
            {/* Allow non-numeric, non-X columns as Y too — mirrors DataChartView */}
            {stringColumns.filter((c) => c !== xAxis).map((col) => (
              <label
                key={col}
                className="flex cursor-pointer items-center gap-1.5 rounded px-1.5 py-1 text-xs hover:bg-muted/50"
              >
                <input
                  type="checkbox"
                  checked={yAxes.includes(col)}
                  onChange={() => toggleY(col)}
                  className="h-3 w-3 rounded"
                />
                <span className="truncate text-muted-foreground">{col}</span>
              </label>
            ))}
          </div>
        </div>
      </div>

      {/* Chart canvas */}
      <div className="flex-1 overflow-hidden p-4">
        {renderChart()}
      </div>
    </div>
  );
}
