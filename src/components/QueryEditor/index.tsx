import { useState, useRef, useCallback, useEffect, useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { ScrollArea } from '@/components/ui/scroll-area';
import Editor, { OnMount, BeforeMount } from '@monaco-editor/react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { QueryResult } from '@/types/database';
import { Play, Eraser, Download, Loader2, Clock, Rows3, FolderOpen, Save, RefreshCw, BarChart3 } from 'lucide-react';
import type { editor } from 'monaco-editor';
import { cn } from '@/lib/utils';
import { useTabStore } from '@/stores/tabStore';
import { useThemeStore } from '@/stores/themeStore';
import { configureMonacoForDialect, SqlDialect } from '@/lib/monacoSetup';
import { open, save } from '@tauri-apps/plugin-dialog';
import {
  BarChart, Bar, XAxis, YAxis, CartesianGrid, Tooltip, Legend,
  ResponsiveContainer,
} from 'recharts';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { readTextFile, writeTextFile } from '@tauri-apps/plugin-fs';

interface Props {
  connectionId: string;
  database: string;
  tabKey: string;
  dbType?: string;
  initialContent?: string;
}

export default function QueryEditor({ connectionId, database, tabKey, dbType, initialContent }: Props) {
  const { t } = useTranslation();
  const [result, setResult] = useState<QueryResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showChart, setShowChart] = useState(false);
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const updateTabContent = useTabStore((s) => s.updateTabContent);
  const resolvedTheme = useThemeStore((s) => s.resolvedTheme);

  useEffect(() => {
    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
    };
  }, []);

  const handleContentChange = useCallback(
    (value: string | undefined) => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
      debounceTimerRef.current = setTimeout(() => {
        updateTabContent(tabKey, value ?? '');
      }, 500);
    },
    [tabKey, updateTabContent]
  );

  const handleBeforeMount: BeforeMount = (monaco) => {
    if (dbType) {
      configureMonacoForDialect(monaco, dbType as SqlDialect);
    }
  };

  const handleEditorMount: OnMount = (editor, monaco) => {
    editorRef.current = editor;
    editor.addAction({
      id: 'execute-query',
      label: 'Execute Query',
      keybindings: [2048 | 3],
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

  const handleExecute = useCallback(async () => {
    const ed = editorRef.current;
    if (!ed) return;
    const selection = ed.getSelection();
    let sql = '';
    if (selection && !selection.isEmpty()) {
      sql = ed.getModel()?.getValueInRange(selection) || '';
    } else {
      sql = ed.getValue();
    }
    if (!sql.trim()) return;

    setLoading(true);
    setError(null);
    try {
      const res = await invoke<QueryResult>('execute_query', {
        connectionId, database, sql: sql.trim(),
      });
      setResult(res);
    } catch (e: any) {
      setError(typeof e === 'string' ? e : e.message || '查询执行失败');
      setResult(null);
    } finally {
      setLoading(false);
    }
  }, [connectionId, database]);

  const handleClear = () => {
    editorRef.current?.setValue('');
    setResult(null);
    setError(null);
  };

  const handleExport = () => {
    if (!result || result.rows.length === 0) return;
    const headers = result.columns.map((c) => c.name).join(',');
    const rows = result.rows.map((row) =>
      row.map((cell) => {
        const str = cell === null ? '' : String(cell);
        return str.includes(',') || str.includes('"') ? `"${str.replace(/"/g, '""')}"` : str;
      }).join(',')
    ).join('\n');
    const blob = new Blob(['\uFEFF' + headers + '\n' + rows], { type: 'text/csv;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `query_result_${Date.now()}.csv`;
    a.click();
    URL.revokeObjectURL(url);
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
        <Button variant="ghost" size="sm" onClick={handleClear}>
          <Eraser className="mr-1.5 h-3.5 w-3.5" />
          {t('query.clear')}
        </Button>
        <Button variant="ghost" size="sm" onClick={handleExport} disabled={!result || result.rows.length === 0}>
          <Download className="mr-1.5 h-3.5 w-3.5" />
          {t('query.export')}
        </Button>
        <Button variant="ghost" size="sm" onClick={handleOpenFile}>
          <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
          打开
        </Button>
        <Button variant="ghost" size="sm" onClick={handleSaveFile}>
          <Save className="mr-1.5 h-3.5 w-3.5" />
          保存
        </Button>
        <div className="ml-auto">
          <Badge variant="outline" className="font-mono text-[11px]">{database}</Badge>
        </div>
      </div>

      {/* Editor */}
      <div className="h-[250px] min-h-[120px] shrink-0 border-b">
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

      {/* Result toolbar */}
      {result && result.columns.length > 0 && (
        <div className="flex shrink-0 items-center gap-2 border-b bg-muted/20 px-4 py-1">
          <span className="text-xs font-medium text-muted-foreground">{t('query.result')}</span>
          <div className="mx-1 h-3 w-px bg-border" />
          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={handleExecute} disabled={loading}>
            <RefreshCw className={cn("mr-1 h-3 w-3", loading && "animate-spin")} />
            刷新
          </Button>
          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={handleExport} disabled={result.rows.length === 0}>
            <Download className="mr-1 h-3 w-3" />
            导出
          </Button>
          <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={() => {
            // Open an inline chart view for the query result (stored in state)
            setShowChart(prev => !prev);
          }}>
            <BarChart3 className="mr-1 h-3 w-3" />
            图表
          </Button>
          <span className="ml-auto text-[10px] text-muted-foreground">
            {result.rows.length} {t('query.rows')} | {result.execution_time_ms}ms
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
        ) : error ? (
          <div className="bg-red-50/50 px-4 py-3 text-sm text-red-600">{error}</div>
        ) : result ? (
          result.columns.length > 0 ? (
            showChart ? (
              <QueryChartView result={result} />
            ) : (
            <ScrollArea className="h-full">
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead className="sticky top-0 z-10">
                    <tr className="border-b bg-muted/60">
                      <th className="w-12 px-3 py-2 text-center text-xs font-medium text-muted-foreground">#</th>
                      {result.columns.map((col) => (
                        <th key={col.name} className="whitespace-nowrap border-l px-4 py-2 text-left text-xs font-medium">
                          {col.name}
                          <span className="ml-2 font-normal text-muted-foreground">{col.data_type}</span>
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {result.rows.map((row, ri) => (
                      <tr key={ri} className="border-b transition-colors hover:bg-muted/30">
                        <td className="px-3 py-1.5 text-center text-xs text-muted-foreground">{ri + 1}</td>
                        {row.map((cell, ci) => (
                          <td
                            key={ci}
                            className={cn(
                              "max-w-[300px] truncate border-l px-4 py-1.5 font-mono text-xs",
                              cell === null && "italic text-muted-foreground/50"
                            )}
                          >
                            {cell === null ? 'NULL' : typeof cell === 'object' ? JSON.stringify(cell) : String(cell)}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </ScrollArea>
            )
          ) : (
            <div className="px-4 py-3 text-sm text-emerald-600">
              {t('query.affected')}: {result.affected_rows} {t('query.rows')}
            </div>
          )
        ) : (
          <div className="flex h-full flex-col items-center justify-center gap-1 text-muted-foreground">
            <span className="text-sm">输入 SQL 并按 Ctrl+Enter 执行</span>
          </div>
        )}
      </div>

    </div>
  );
}

/* Inline chart view for query results */
const CHART_COLORS = ['hsl(210,80%,55%)', 'hsl(150,70%,45%)', 'hsl(350,75%,55%)', 'hsl(40,85%,55%)', 'hsl(270,65%,55%)', 'hsl(180,60%,45%)'];

function QueryChartView({ result }: { result: QueryResult }) {
  const [xAxis, setXAxis] = useState<string>('');
  const [yAxes, setYAxes] = useState<string[]>([]);

  const columns = result.columns;

  // Auto-detect numeric columns
  const numericCols = useMemo(() => {
    return columns.filter((col) => {
      for (let i = 0; i < Math.min(10, result.rows.length); i++) {
        const idx = columns.indexOf(col);
        const val = result.rows[i]?.[idx];
        if (val !== null && val !== undefined && !isNaN(Number(val))) return true;
      }
      return false;
    }).map(c => c.name);
  }, [columns, result.rows]);

  const stringCols = columns.map(c => c.name).filter(n => !numericCols.includes(n));

  useEffect(() => {
    if (!xAxis && stringCols.length > 0) setXAxis(stringCols[0]);
    else if (!xAxis && columns.length > 0) setXAxis(columns[0].name);
    if (yAxes.length === 0 && numericCols.length > 0) setYAxes([numericCols[0]]);
  }, [columns, numericCols, stringCols, xAxis, yAxes.length]);

  const chartData = useMemo(() =>
    result.rows.map(row => {
      const obj: Record<string, any> = {};
      columns.forEach((col, i) => { obj[col.name] = row[i]; });
      return obj;
    }),
    [result.rows, columns]
  );

  const toggleY = (col: string) => {
    setYAxes(prev => prev.includes(col) ? prev.filter(c => c !== col) : [...prev, col]);
  };

  return (
    <div className="flex h-full">
      <div className="w-44 shrink-0 overflow-y-auto border-r p-3">
        <div className="mb-3">
          <label className="mb-1 block text-[10px] font-medium text-muted-foreground">X 轴</label>
          <Select value={xAxis} onValueChange={setXAxis}>
            <SelectTrigger className="h-7 text-xs"><SelectValue /></SelectTrigger>
            <SelectContent>{columns.map(c => <SelectItem key={c.name} value={c.name}>{c.name}</SelectItem>)}</SelectContent>
          </Select>
        </div>
        <div>
          <label className="mb-1 block text-[10px] font-medium text-muted-foreground">Y 轴</label>
          {numericCols.map(col => (
            <label key={col} className="flex items-center gap-1.5 py-0.5 text-xs">
              <input type="checkbox" checked={yAxes.includes(col)} onChange={() => toggleY(col)} className="h-3 w-3 rounded" />
              {col}
            </label>
          ))}
          {numericCols.length === 0 && <p className="text-[10px] text-muted-foreground">无数值列</p>}
        </div>
      </div>
      <div className="flex-1 p-4">
        {yAxes.length > 0 && xAxis ? (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} tick={{ fontSize: 11 }} />
              <YAxis tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={{ background: 'var(--popover)', border: '1px solid var(--border)', borderRadius: '6px', fontSize: 12 }} />
              <Legend />
              {yAxes.map((col, i) => (
                <Bar key={col} dataKey={col} fill={CHART_COLORS[i % CHART_COLORS.length]} />
              ))}
            </BarChart>
          </ResponsiveContainer>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">选择 Y 轴数值列以显示图表</div>
        )}
      </div>
    </div>
  );
}
