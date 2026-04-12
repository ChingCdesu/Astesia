import { useState, useEffect, useCallback, useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { ScrollArea } from '@/components/ui/scroll-area';
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { QueryResult } from '@/types/database';
import {
  BarChart, Bar, LineChart, Line, AreaChart, Area,
  ScatterChart, Scatter, PieChart, Pie, Cell,
  XAxis, YAxis, CartesianGrid, Tooltip, Legend,
  ResponsiveContainer,
} from 'recharts';
import {
  RefreshCw, Loader2, PanelLeftClose, PanelLeftOpen, BarChart3, Table2,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { useTabStore } from '@/stores/tabStore';

const COLORS = [
  'hsl(210, 80%, 55%)',
  'hsl(150, 70%, 45%)',
  'hsl(350, 75%, 55%)',
  'hsl(40, 85%, 55%)',
  'hsl(270, 65%, 55%)',
  'hsl(180, 60%, 45%)',
  'hsl(320, 70%, 55%)',
  'hsl(20, 80%, 55%)',
];

type ChartType = 'bar' | 'line' | 'area' | 'scatter' | 'pie';

interface Props {
  connectionId: string;
  database: string;
  table: string;
}

export default function DataChartView({ connectionId, database, table }: Props) {
  const { t } = useTranslation();

  const [result, setResult] = useState<QueryResult | null>(null);
  const [loading, setLoading] = useState(false);

  const [chartType, setChartType] = useState<ChartType>('bar');
  const [xAxis, setXAxis] = useState<string>('');
  const [yAxes, setYAxes] = useState<string[]>([]);
  const [configCollapsed, setConfigCollapsed] = useState(false);
  const [filterWhere, setFilterWhere] = useState('');
  const [appliedFilter, setAppliedFilter] = useState('');

  // Load ALL data (fetch pages until exhausted)
  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      if (appliedFilter) {
        // Use custom query with WHERE clause
        const tableName = table.includes('.')
          ? `"${table.split('.')[0]}"."${table.split('.')[1]}"`
          : `"${table}"`;
        const sql = `SELECT * FROM ${tableName} WHERE ${appliedFilter}`;
        const data = await invoke<QueryResult>('execute_query', {
          connectionId, database, sql,
        });
        setResult(data);
      } else {
        // Batch loading without filter
        const batchSize = 1000;
        let page = 1;
        let allRows: any[][] = [];
        let columns: QueryResult['columns'] = [];

        while (true) {
          const data = await invoke<QueryResult>('get_table_data', {
            connectionId, database, table, page, pageSize: batchSize,
          });
          if (page === 1) columns = data.columns;
          allRows = allRows.concat(data.rows);
          if (data.rows.length < batchSize) break;
          page++;
        }

        setResult({ columns, rows: allRows, affected_rows: 0, execution_time_ms: 0 });
      }
    } catch (e: any) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [connectionId, database, table, appliedFilter]);

  useEffect(() => { loadData(); }, [loadData]);

  // Detect numeric and string columns
  const { numericColumns, stringColumns } = useMemo(() => {
    if (!result || result.rows.length === 0) {
      return { numericColumns: [] as string[], stringColumns: [] as string[] };
    }

    const numeric: string[] = [];
    const str: string[] = [];
    const sampleSize = Math.min(result.rows.length, 10);

    result.columns.forEach((col, colIndex) => {
      let numCount = 0;
      for (let i = 0; i < sampleSize; i++) {
        const val = result.rows[i][colIndex];
        if (val === null || val === undefined) continue;
        if (typeof val === 'number' || (typeof val === 'string' && val !== '' && !isNaN(Number(val)))) {
          numCount++;
        }
      }
      // If more than half of sampled non-null values are numeric, treat as numeric
      if (numCount > sampleSize / 2) {
        numeric.push(col.name);
      } else {
        str.push(col.name);
      }
    });

    return { numericColumns: numeric, stringColumns: str };
  }, [result]);

  // Auto-select axes when data changes
  useEffect(() => {
    if (stringColumns.length > 0 && !xAxis) {
      setXAxis(stringColumns[0]);
    } else if (numericColumns.length > 0 && !xAxis && stringColumns.length === 0) {
      // Fallback: use first column as X axis
      setXAxis(result?.columns[0]?.name || '');
    }
    if (numericColumns.length > 0 && yAxes.length === 0) {
      setYAxes(numericColumns.slice(0, 2));
    }
  }, [numericColumns, stringColumns, result]);

  // Transform data for recharts — aggregate rows with duplicate X values
  const chartData = useMemo(() => {
    if (!result || !xAxis) return [];

    const xColIndex = result.columns.findIndex(c => c.name === xAxis);
    if (xColIndex < 0) return [];

    // Always aggregate by X value when X is a string column
    if (stringColumns.includes(xAxis)) {
      // Aggregate: group by X value, sum numeric Y columns
      const grouped = new Map<string, { count: number; sums: Record<string, number> }>();
      result.rows.forEach(row => {
        const xVal = String(row[xColIndex] ?? '');
        if (!grouped.has(xVal)) {
          grouped.set(xVal, { count: 0, sums: {} });
        }
        const entry = grouped.get(xVal)!;
        entry.count++;
        result.columns.forEach((col, i) => {
          if (col.name === xAxis) return;
          const val = row[i];
          const num = typeof val === 'number' ? val : (typeof val === 'string' && val !== '' && !isNaN(Number(val)) ? Number(val) : 0);
          entry.sums[col.name] = (entry.sums[col.name] || 0) + num;
        });
      });

      return Array.from(grouped.entries()).map(([xVal, entry]) => {
        const obj: Record<string, any> = { [xAxis]: xVal };
        Object.entries(entry.sums).forEach(([col, sum]) => {
          obj[col] = Math.round(sum * 100) / 100; // SUM, rounded to 2 decimals
        });
        obj['_count'] = entry.count;
        return obj;
      });
    }

    // No significant duplicates — use raw rows
    return result.rows.map(row => {
      const obj: Record<string, any> = {};
      result.columns.forEach((col, i) => {
        const val = row[i];
        obj[col.name] = typeof val === 'string' && !isNaN(Number(val)) && val !== ''
          ? Number(val)
          : val;
      });
      return obj;
    });
  }, [result, xAxis, stringColumns]);

  // All column names for X axis selection (both string and numeric)
  const allColumns = useMemo(() => {
    if (!result) return [];
    return result.columns.map(c => c.name);
  }, [result]);

  // Toggle Y axis column
  const toggleYAxis = (col: string) => {
    setYAxes(prev =>
      prev.includes(col)
        ? prev.filter(c => c !== col)
        : [...prev, col]
    );
  };

  const tooltipStyle = {
    background: 'var(--popover)',
    border: '1px solid var(--border)',
    borderRadius: '6px',
    color: 'var(--popover-foreground)',
  };

  const renderChart = () => {
    if (!result || chartData.length === 0 || !xAxis || yAxes.length === 0) {
      return (
        <div className="flex h-full items-center justify-center text-muted-foreground">
          <div className="text-center">
            <BarChart3 className="mx-auto mb-2 h-10 w-10 opacity-30" />
            <p className="text-sm">
              {numericColumns.length === 0
                ? t('chart.noNumericColumns')
                : t('chart.noData')}
            </p>
          </div>
        </div>
      );
    }

    switch (chartType) {
      case 'bar':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} className="text-xs" tick={{ fontSize: 11 }} />
              <YAxis className="text-xs" tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
              {yAxes.map((col, i) => (
                <Bar key={col} dataKey={col} fill={COLORS[i % COLORS.length]} />
              ))}
            </BarChart>
          </ResponsiveContainer>
        );

      case 'line':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <LineChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} className="text-xs" tick={{ fontSize: 11 }} />
              <YAxis className="text-xs" tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
              {yAxes.map((col, i) => (
                <Line
                  key={col}
                  type="monotone"
                  dataKey={col}
                  stroke={COLORS[i % COLORS.length]}
                  strokeWidth={2}
                  dot={{ r: 3 }}
                />
              ))}
            </LineChart>
          </ResponsiveContainer>
        );

      case 'area':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={chartData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} className="text-xs" tick={{ fontSize: 11 }} />
              <YAxis className="text-xs" tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} />
              <Legend />
              {yAxes.map((col, i) => (
                <Area
                  key={col}
                  type="monotone"
                  dataKey={col}
                  fill={COLORS[i % COLORS.length]}
                  stroke={COLORS[i % COLORS.length]}
                  fillOpacity={0.3}
                />
              ))}
            </AreaChart>
          </ResponsiveContainer>
        );

      case 'scatter':
        return (
          <ResponsiveContainer width="100%" height="100%">
            <ScatterChart>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey={xAxis} name={xAxis} className="text-xs" tick={{ fontSize: 11 }} />
              <YAxis dataKey={yAxes[0]} name={yAxes[0]} className="text-xs" tick={{ fontSize: 11 }} />
              <Tooltip contentStyle={tooltipStyle} cursor={{ strokeDasharray: '3 3' }} />
              <Legend />
              <Scatter name={`${xAxis} / ${yAxes[0]}`} data={chartData} fill={COLORS[0]} />
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
                label={({ name, percent }: any) => `${name ?? ''}: ${((percent ?? 0) * 100).toFixed(0)}%`}
              >
                {chartData.map((_, i) => (
                  <Cell key={i} fill={COLORS[i % COLORS.length]} />
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

  const chartTypeOptions: { value: ChartType; label: string }[] = [
    { value: 'bar', label: t('chart.bar') },
    { value: 'line', label: t('chart.line') },
    { value: 'area', label: t('chart.area') },
    { value: 'scatter', label: t('chart.scatter') },
    { value: 'pie', label: t('chart.pie') },
  ];

  return (
    <div className="flex h-full flex-col">
      {/* Toolbar */}
      <div className="flex shrink-0 items-center gap-2 border-b bg-muted/30 px-4 py-2">
        <Select value={chartType} onValueChange={(v) => setChartType(v as ChartType)}>
          <SelectTrigger className="h-8 w-[140px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {chartTypeOptions.map(opt => (
              <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
            ))}
          </SelectContent>
        </Select>

        <input
          className="flex-1 h-7 rounded border border-input bg-transparent px-2 text-xs font-mono select-text focus:outline-none focus:ring-1 focus:ring-primary"
          placeholder="WHERE ..."
          value={filterWhere}
          onChange={(e) => setFilterWhere(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter') { setAppliedFilter(filterWhere); } }}
        />

        <Button variant="ghost" size="sm" onClick={loadData} disabled={loading}>
          <RefreshCw className={cn("mr-1.5 h-3.5 w-3.5", loading && "animate-spin")} />
          {t('table.refresh')}
        </Button>

        <Button
          variant="ghost"
          size="sm"
          onClick={() => setConfigCollapsed(!configCollapsed)}
        >
          {configCollapsed
            ? <PanelLeftOpen className="mr-1.5 h-3.5 w-3.5" />
            : <PanelLeftClose className="mr-1.5 h-3.5 w-3.5" />
          }
          {t('chart.config')}
        </Button>

        <Button variant="ghost" size="sm" onClick={() => {
          const displayName = table.includes('.') ? table.split('.').pop()! : table;
          useTabStore.getState().addTab({
            key: `data-${connectionId}-${database}-${table}`,
            label: `${displayName} [${database}]`,
            type: 'table-data',
            connectionId,
            database,
            table,
          });
        }}>
          <Table2 className="mr-1.5 h-3.5 w-3.5" />
          返回表格
        </Button>

        <div className="ml-auto flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="font-mono">{database}.{table}</span>
        </div>
      </div>

      {/* Main area */}
      <div className="flex flex-1 overflow-hidden">
        {/* Config panel */}
        {!configCollapsed && (
          <div className="w-[200px] shrink-0 border-r bg-muted/20">
            <ScrollArea className="h-full">
              <div className="flex flex-col gap-4 p-3">
                {/* X Axis */}
                <div>
                  <label className="mb-1.5 block text-xs font-medium text-muted-foreground">
                    {t('chart.xAxis')}
                  </label>
                  <Select value={xAxis} onValueChange={setXAxis}>
                    <SelectTrigger className="h-8 text-xs">
                      <SelectValue placeholder={t('chart.selectColumns')} />
                    </SelectTrigger>
                    <SelectContent>
                      {allColumns.map(col => (
                        <SelectItem key={col} value={col}>{col}</SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>

                {/* Y Axis (multi-select checkboxes) */}
                <div>
                  <label className="mb-1.5 block text-xs font-medium text-muted-foreground">
                    {t('chart.yAxis')}
                  </label>
                  <div className="flex flex-col gap-1">
                    {chartData.length > 0 && chartData[0]?.['_count'] !== undefined && (
                      <label className="flex cursor-pointer items-center gap-2 rounded bg-muted/30 px-1.5 py-1 text-xs hover:bg-muted/50">
                        <input type="checkbox" checked={yAxes.includes('_count')} onChange={() => toggleYAxis('_count')} className="h-3.5 w-3.5 rounded border-input accent-primary" />
                        <span className="truncate italic">_count (聚合计数)</span>
                      </label>
                    )}
                    {numericColumns.length > 0 ? (
                      numericColumns.map(col => (
                        <label
                          key={col}
                          className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-xs hover:bg-muted/50"
                        >
                          <input
                            type="checkbox"
                            checked={yAxes.includes(col)}
                            onChange={() => toggleYAxis(col)}
                            className="h-3.5 w-3.5 rounded border-input accent-primary"
                          />
                          <span className="truncate">{col}</span>
                        </label>
                      ))
                    ) : (
                      <p className="text-xs italic text-muted-foreground">
                        {t('chart.noNumericColumns')}
                      </p>
                    )}
                  </div>
                </div>

                {/* Also allow selecting non-numeric columns for Y axis if user wants */}
                {stringColumns.filter(c => c !== xAxis).length > 0 && (
                  <div>
                    <label className="mb-1.5 block text-xs font-medium text-muted-foreground/60">
                      {t('chart.selectColumns')}
                    </label>
                    <div className="flex flex-col gap-1">
                      {stringColumns.filter(c => c !== xAxis).map(col => (
                        <label
                          key={col}
                          className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-xs hover:bg-muted/50"
                        >
                          <input
                            type="checkbox"
                            checked={yAxes.includes(col)}
                            onChange={() => toggleYAxis(col)}
                            className="h-3.5 w-3.5 rounded border-input accent-primary"
                          />
                          <span className="truncate text-muted-foreground">{col}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            </ScrollArea>
          </div>
        )}

        {/* Chart area */}
        <div className="flex-1 overflow-hidden p-4">
          {loading && !result ? (
            <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
              <span>{t('common.loading')}</span>
            </div>
          ) : (
            <div className="h-full w-full">
              {renderChart()}
            </div>
          )}
        </div>
      </div>

      {/* Status bar */}
      <div className="flex shrink-0 items-center border-t bg-muted/30 px-4 py-1.5">
        <span className="text-xs text-muted-foreground">
          {result ? `${result.rows.length} ${t('query.rows')}` : ''}
        </span>
      </div>
    </div>
  );
}
