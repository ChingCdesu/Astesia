import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { ScrollArea } from '@/components/ui/scroll-area';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { QueryResult } from '@/types/database';
import { CellChange, NewRow } from '@/types/editing';
import {
  RefreshCw, Download, Loader2, ChevronLeft, ChevronRight,
  Plus, Trash2, Save, Undo2, AlertTriangle, ClipboardPaste, BarChart3,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { useTabStore } from '@/stores/tabStore';

function parseClipboardData(text: string): string[][] {
  // Handle both CSV (comma-separated) and TSV (tab-separated, from Excel)
  const lines = text.trim().split(/\r?\n/);
  return lines.map(line => {
    // If contains tabs, treat as TSV (Excel format)
    if (line.includes('\t')) {
      return line.split('\t').map(cell => cell.trim());
    }
    // Otherwise parse as CSV (handle quoted fields)
    const cells: string[] = [];
    let current = '';
    let inQuotes = false;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (inQuotes) {
        if (ch === '"' && line[i + 1] === '"') {
          current += '"';
          i++;
        } else if (ch === '"') {
          inQuotes = false;
        } else {
          current += ch;
        }
      } else {
        if (ch === '"') {
          inQuotes = true;
        } else if (ch === ',') {
          cells.push(current.trim());
          current = '';
        } else {
          current += ch;
        }
      }
    }
    cells.push(current.trim());
    return cells;
  });
}

interface Props {
  connectionId: string;
  database: string;
  table: string;
}

interface HistoryEntry {
  type: 'edit' | 'delete' | 'insert';
  key?: string;
  change?: CellChange;
  rowIndices?: number[];
  newRowIndex?: number;
}

export default function DataGrid({ connectionId, database, table }: Props) {
  const { t } = useTranslation();
  const [result, setResult] = useState<QueryResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize] = useState(100);

  // Editing state
  const [editingCell, setEditingCell] = useState<{ row: number; col: number } | null>(null);
  const [editValue, setEditValue] = useState('');
  const [pendingChanges, setPendingChanges] = useState<Map<string, CellChange>>(new Map());
  const [newRows, setNewRows] = useState<NewRow[]>([]);
  const [deletedRows, setDeletedRows] = useState<Set<number>>(new Set());
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [changeHistory, setChangeHistory] = useState<HistoryEntry[]>([]);
  const [lastSelectedRow, setLastSelectedRow] = useState<number | null>(null);

  const containerRef = useRef<HTMLDivElement>(null);
  const editInputRef = useRef<HTMLInputElement>(null);

  // Primary key detection
  const primaryKeyColumn = useMemo(() => {
    if (!result) return null;
    return result.columns.find((c) => c.is_primary_key) ?? null;
  }, [result]);

  const hasPrimaryKey = primaryKeyColumn !== null;

  const hasChanges = pendingChanges.size > 0 || newRows.length > 0 || deletedRows.size > 0;

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [data, columnMeta] = await Promise.all([
        invoke<QueryResult>('get_table_data', { connectionId, database, table, page, pageSize }),
        invoke<import('@/types/database').ColumnInfo[]>('get_columns', { connectionId, database, table }),
      ]);
      // Merge PK info from get_columns into result columns (get_table_data doesn't have PK metadata)
      const pkColumns = new Set(columnMeta.filter(c => c.is_primary_key).map(c => c.name));
      data.columns = data.columns.map(col => ({
        ...col,
        is_primary_key: pkColumns.has(col.name),
      }));
      setResult(data);
    } catch (e: any) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [connectionId, database, table, page, pageSize]);

  useEffect(() => { loadData(); }, [loadData]);

  // Reset editing state when table/page changes
  useEffect(() => {
    setEditingCell(null);
    setPendingChanges(new Map());
    setNewRows([]);
    setDeletedRows(new Set());
    setSelectedRows(new Set());
    setChangeHistory([]);
    setLastSelectedRow(null);
  }, [connectionId, database, table, page]);

  // Focus edit input when editing cell changes
  useEffect(() => {
    if (editingCell && editInputRef.current) {
      editInputRef.current.focus();
      editInputRef.current.select();
    }
  }, [editingCell]);

  // Keyboard shortcuts handler (Ctrl+Z, Ctrl+V, Ctrl+C)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'z') {
        e.preventDefault();
        handleUndo();
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'v') {
        e.preventDefault();
        handlePaste();
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'c') {
        if (selectedRows.size > 0 && result) {
          e.preventDefault();
          handleCopyRows();
        }
      }
    };
    const el = containerRef.current;
    if (el) {
      el.addEventListener('keydown', handleKeyDown);
      return () => el.removeEventListener('keydown', handleKeyDown);
    }
  });

  const handleUndo = () => {
    setChangeHistory((prev) => {
      if (prev.length === 0) return prev;
      const next = [...prev];
      const last = next.pop()!;

      if (last.type === 'edit' && last.key && last.change) {
        setPendingChanges((map) => {
          const newMap = new Map(map);
          newMap.delete(last.key!);
          return newMap;
        });
      } else if (last.type === 'delete' && last.rowIndices) {
        setDeletedRows((set) => {
          const newSet = new Set(set);
          last.rowIndices!.forEach((i) => newSet.delete(i));
          return newSet;
        });
      } else if (last.type === 'insert' && last.newRowIndex !== undefined) {
        setNewRows((rows) => rows.filter((_, i) => i !== last.newRowIndex));
      }

      return next;
    });
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
    a.download = `${table}_${Date.now()}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  // Cell double-click to start editing
  const handleCellDoubleClick = (rowIndex: number, colIndex: number) => {
    if (!hasPrimaryKey) return;
    if (deletedRows.has(rowIndex)) return;

    const col = result!.columns[colIndex];
    const key = `${rowIndex}-${col.name}`;
    const existing = pendingChanges.get(key);
    const currentValue = existing ? existing.newValue : result!.rows[rowIndex][colIndex];

    setEditingCell({ row: rowIndex, col: colIndex });
    setEditValue(currentValue === null ? '' : String(currentValue));
  };

  // Commit cell edit
  const commitEdit = () => {
    if (!editingCell || !result) return;

    const { row, col } = editingCell;
    const column = result.columns[col];
    const originalValue = result.rows[row][col];
    const key = `${row}-${column.name}`;

    // Parse the new value
    let newValue: any = editValue;
    if (editValue === '' && column.nullable) {
      newValue = null;
    } else if (editValue === 'NULL' || editValue === 'null') {
      newValue = null;
    }

    // Check if value actually changed from original
    const existingChange = pendingChanges.get(key);
    const baseValue = existingChange ? existingChange.oldValue : originalValue;
    const baseStr = baseValue === null ? '' : String(baseValue);
    const newStr = newValue === null ? '' : String(newValue);

    if (baseStr === newStr && baseValue === null === (newValue === null)) {
      // Reverted to original - remove the pending change
      if (existingChange) {
        setPendingChanges((map) => {
          const newMap = new Map(map);
          newMap.delete(key);
          return newMap;
        });
      }
    } else {
      const change: CellChange = {
        rowIndex: row,
        columnName: column.name,
        oldValue: existingChange ? existingChange.oldValue : originalValue,
        newValue,
      };
      setPendingChanges((map) => {
        const newMap = new Map(map);
        newMap.set(key, change);
        return newMap;
      });
      setChangeHistory((prev) => [...prev, { type: 'edit', key, change }]);
    }

    setEditingCell(null);
  };

  const cancelEdit = () => {
    setEditingCell(null);
  };

  // Row selection
  const handleRowSelect = (rowIndex: number, shiftKey: boolean) => {
    if (shiftKey && lastSelectedRow !== null) {
      const start = Math.min(lastSelectedRow, rowIndex);
      const end = Math.max(lastSelectedRow, rowIndex);
      setSelectedRows((prev) => {
        const newSet = new Set(prev);
        for (let i = start; i <= end; i++) {
          newSet.add(i);
        }
        return newSet;
      });
    } else {
      setSelectedRows((prev) => {
        const newSet = new Set(prev);
        if (newSet.has(rowIndex)) {
          newSet.delete(rowIndex);
        } else {
          newSet.add(rowIndex);
        }
        return newSet;
      });
    }
    setLastSelectedRow(rowIndex);
  };

  // Add new row
  const handleAddRow = () => {
    if (!result || !hasPrimaryKey) return;
    const emptyValues: Record<string, any> = {};
    result.columns.forEach((col) => {
      emptyValues[col.name] = null;
    });
    const newRowIndex = newRows.length;
    setNewRows((prev) => [...prev, { values: emptyValues }]);
    setChangeHistory((prev) => [...prev, { type: 'insert', newRowIndex }]);
  };

  // Delete selected rows
  const handleDeleteRows = () => {
    if (!hasPrimaryKey || selectedRows.size === 0) return;
    const indices = Array.from(selectedRows).filter((i) => !deletedRows.has(i));
    if (indices.length === 0) return;

    setDeletedRows((prev) => {
      const newSet = new Set(prev);
      indices.forEach((i) => newSet.add(i));
      return newSet;
    });
    setChangeHistory((prev) => [...prev, { type: 'delete', rowIndices: indices }]);
    setSelectedRows(new Set());
  };

  // Save all changes
  const handleSave = async () => {
    if (!result || !primaryKeyColumn) return;
    setSaving(true);

    try {
      // 1. Apply updates
      for (const [, change] of pendingChanges) {
        const pkValue = result.rows[change.rowIndex][
          result.columns.findIndex((c) => c.name === primaryKeyColumn.name)
        ];
        await invoke('update_row', {
          connectionId,
          database,
          table,
          primaryKeyColumn: primaryKeyColumn.name,
          primaryKeyValue: pkValue,
          column: change.columnName,
          newValue: change.newValue,
        });
      }

      // 2. Insert new rows
      for (const newRow of newRows) {
        const cols = Object.keys(newRow.values).filter((k) => newRow.values[k] !== null);
        const vals = cols.map((k) => newRow.values[k]);
        if (cols.length > 0) {
          await invoke('insert_row', {
            connectionId,
            database,
            table,
            columns: cols,
            values: vals,
          });
        }
      }

      // 3. Delete rows
      if (deletedRows.size > 0) {
        const pkColIndex = result.columns.findIndex((c) => c.name === primaryKeyColumn.name);
        const pkValues = Array.from(deletedRows).map((i) => result.rows[i][pkColIndex]);
        await invoke('delete_rows', {
          connectionId,
          database,
          table,
          primaryKeyColumn: primaryKeyColumn.name,
          primaryKeyValues: pkValues,
        });
      }

      // Reset state and reload
      setPendingChanges(new Map());
      setNewRows([]);
      setDeletedRows(new Set());
      setSelectedRows(new Set());
      setChangeHistory([]);
      setEditingCell(null);
      await loadData();
    } catch (e: any) {
      console.error('保存失败:', e);
    } finally {
      setSaving(false);
    }
  };

  // Discard all changes
  const handleDiscard = () => {
    setPendingChanges(new Map());
    setNewRows([]);
    setDeletedRows(new Set());
    setSelectedRows(new Set());
    setChangeHistory([]);
    setEditingCell(null);
  };

  // Paste CSV/TSV data from clipboard
  const handlePaste = async () => {
    if (!result || !hasPrimaryKey) return;
    try {
      const text = await navigator.clipboard.readText();
      if (!text.trim()) return;

      const parsedRows = parseClipboardData(text);
      if (parsedRows.length === 0) return;

      // Detect if first row is headers (match column names)
      const columns = result.columns;
      let dataRows = parsedRows;
      let colMapping: number[] = [];

      const firstRow = parsedRows[0];
      const headerMatch = firstRow.filter(cell =>
        columns.some(col => col.name.toLowerCase() === cell.toLowerCase())
      );

      if (headerMatch.length > firstRow.length * 0.5) {
        // First row looks like headers — map them to column indices
        colMapping = firstRow.map(header =>
          columns.findIndex(col => col.name.toLowerCase() === header.toLowerCase())
        );
        dataRows = parsedRows.slice(1);
      } else {
        // No headers — map sequentially to columns
        colMapping = firstRow.map((_, i) => i < columns.length ? i : -1);
      }

      // Create new rows from parsed data
      const newRowsToAdd: NewRow[] = dataRows.map(row => {
        const values: Record<string, any> = {};
        columns.forEach(col => { values[col.name] = null; });

        row.forEach((cell, i) => {
          const colIdx = colMapping[i];
          if (colIdx >= 0 && colIdx < columns.length) {
            values[columns[colIdx].name] = cell === '' || cell === 'NULL' || cell === 'null' ? null : cell;
          }
        });
        return { values };
      });

      if (newRowsToAdd.length > 0) {
        setNewRows(prev => [...prev, ...newRowsToAdd]);
        // Add to history as batch insert
        newRowsToAdd.forEach((_, i) => {
          setChangeHistory(prev => [...prev, { type: 'insert', newRowIndex: newRows.length + i }]);
        });
      }
    } catch (err) {
      console.error('粘贴失败:', err);
    }
  };

  // Copy selected rows as TSV (for Excel compatibility)
  const handleCopyRows = () => {
    if (!result || selectedRows.size === 0) return;
    const headers = result.columns.map(c => c.name).join('\t');
    const rows = Array.from(selectedRows)
      .sort((a, b) => a - b)
      .map(rowIdx =>
        result.columns.map((_, colIdx) => {
          const val = getCellDisplayValue(rowIdx, colIdx);
          return val === null ? '' : String(val);
        }).join('\t')
      )
      .join('\n');
    navigator.clipboard.writeText(headers + '\n' + rows);
  };

  // Get display value for a cell (considering pending changes)
  const getCellDisplayValue = (rowIndex: number, colIndex: number): any => {
    if (!result) return null;
    const col = result.columns[colIndex];
    const key = `${rowIndex}-${col.name}`;
    const change = pendingChanges.get(key);
    return change ? change.newValue : result.rows[rowIndex][colIndex];
  };

  // Check if a cell is dirty
  const isCellDirty = (rowIndex: number, colIndex: number): boolean => {
    if (!result) return false;
    const col = result.columns[colIndex];
    return pendingChanges.has(`${rowIndex}-${col.name}`);
  };

  // Handle new row cell editing
  const handleNewRowCellChange = (newRowIndex: number, columnName: string, value: string) => {
    setNewRows((prev) => {
      const updated = [...prev];
      const row = { ...updated[newRowIndex], values: { ...updated[newRowIndex].values } };
      row.values[columnName] = value === '' ? null : value;
      updated[newRowIndex] = row;
      return updated;
    });
  };

  return (
    <div ref={containerRef} className="flex h-full flex-col" tabIndex={-1}>
      {/* Toolbar */}
      <div className="flex shrink-0 items-center gap-2 border-b bg-muted/30 px-4 py-2">
        <Button variant="ghost" size="sm" onClick={loadData} disabled={loading || saving}>
          <RefreshCw className={cn("mr-1.5 h-3.5 w-3.5", loading && "animate-spin")} />
          {t('table.refresh')}
        </Button>
        <Button variant="ghost" size="sm" onClick={handleExport} disabled={!result || result.rows.length === 0}>
          <Download className="mr-1.5 h-3.5 w-3.5" />
          {t('query.export')}
        </Button>
        <Button variant="ghost" size="sm" onClick={() => {
          useTabStore.getState().addTab({
            key: `chart-${connectionId}-${database}-${table}`,
            label: `${table} [图表]`,
            type: 'data-chart',
            connectionId,
            database,
            table,
          });
        }}>
          <BarChart3 className="mr-1.5 h-3.5 w-3.5" />
          图表
        </Button>

        <div className="mx-1 h-4 w-px bg-border" />

        <Button
          variant="ghost"
          size="sm"
          onClick={handleAddRow}
          disabled={!hasPrimaryKey || saving}
        >
          <Plus className="mr-1.5 h-3.5 w-3.5" />
          新增行
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={handleDeleteRows}
          disabled={!hasPrimaryKey || selectedRows.size === 0 || saving}
        >
          <Trash2 className="mr-1.5 h-3.5 w-3.5" />
          删除行
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={handlePaste}
          disabled={!hasPrimaryKey || saving}
        >
          <ClipboardPaste className="mr-1.5 h-3.5 w-3.5" />
          粘贴
        </Button>

        <div className="mx-1 h-4 w-px bg-border" />

        <Button
          variant="ghost"
          size="sm"
          onClick={handleSave}
          disabled={!hasChanges || saving}
        >
          <Save className={cn("mr-1.5 h-3.5 w-3.5", saving && "animate-spin")} />
          保存更改
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={handleDiscard}
          disabled={!hasChanges || saving}
        >
          <Undo2 className="mr-1.5 h-3.5 w-3.5" />
          放弃更改
        </Button>

        {!hasPrimaryKey && result && (
          <>
            <div className="mx-1 h-4 w-px bg-border" />
            <Badge variant="warning" className="gap-1">
              <AlertTriangle className="h-3 w-3" />
              无主键，不可编辑
            </Badge>
          </>
        )}

        <div className="ml-auto flex items-center gap-1.5">
          <Badge variant="outline" className="font-mono text-[11px]">{database}</Badge>
          <span className="text-muted-foreground">.</span>
          <Badge variant="outline" className="font-mono text-[11px]">{table}</Badge>
        </div>
      </div>

      {/* Table */}
      <div className="flex-1 overflow-hidden">
        {loading && !result ? (
          <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
            <span>{t('common.loading')}</span>
          </div>
        ) : result && result.columns.length > 0 ? (
          <ScrollArea className="h-full">
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="sticky top-0 z-10">
                  <tr className="border-b bg-muted/60">
                    <th className="w-12 px-3 py-2 text-center text-xs font-medium text-muted-foreground">#</th>
                    {result.columns.map((col) => (
                      <th key={col.name} className="whitespace-nowrap border-l px-4 py-2 text-left text-xs font-medium">
                        <div className="flex items-center gap-1.5">
                          {col.is_primary_key && <Badge variant="warning" className="px-1 py-0 text-[9px]">PK</Badge>}
                          <span>{col.name}</span>
                        </div>
                        <span className="mt-0.5 block text-[10px] font-normal text-muted-foreground">{col.data_type}</span>
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {/* Existing rows */}
                  {result.rows.map((row, ri) => {
                    const isDeleted = deletedRows.has(ri);
                    const isSelected = selectedRows.has(ri);

                    return (
                      <tr
                        key={ri}
                        className={cn(
                          "border-b transition-colors",
                          isDeleted
                            ? "bg-red-50 line-through dark:bg-red-950/20"
                            : isSelected
                              ? "bg-blue-50 dark:bg-blue-950/20"
                              : "hover:bg-muted/30",
                        )}
                      >
                        <td
                          className={cn(
                            "px-3 py-1.5 text-center text-xs text-muted-foreground cursor-pointer select-none",
                            isSelected && "font-bold text-blue-600 dark:text-blue-400",
                          )}
                          onClick={(e) => handleRowSelect(ri, e.shiftKey)}
                        >
                          {(page - 1) * pageSize + ri + 1}
                        </td>
                        {row.map((_, ci) => {
                          const cellValue = getCellDisplayValue(ri, ci);
                          const dirty = isCellDirty(ri, ci);
                          const isEditing = editingCell?.row === ri && editingCell?.col === ci;

                          return (
                            <td
                              key={ci}
                              className={cn(
                                "max-w-[300px] border-l px-4 py-1.5 font-mono text-xs",
                                cellValue === null && !isEditing && "italic text-muted-foreground/50",
                                typeof cellValue === 'object' && cellValue !== null && "text-blue-600",
                                dirty && "bg-amber-50 dark:bg-amber-950/20 border-l-2 border-l-amber-400",
                                isDeleted && "opacity-50",
                              )}
                              onDoubleClick={() => handleCellDoubleClick(ri, ci)}
                            >
                              {isEditing ? (
                                <input
                                  ref={editInputRef}
                                  type="text"
                                  className="w-full bg-white dark:bg-zinc-900 border border-blue-400 rounded px-1 py-0.5 text-xs font-mono outline-none select-text"
                                  value={editValue}
                                  onChange={(e) => setEditValue(e.target.value)}
                                  onKeyDown={(e) => {
                                    if (e.key === 'Enter') {
                                      e.preventDefault();
                                      commitEdit();
                                    } else if (e.key === 'Escape') {
                                      e.preventDefault();
                                      cancelEdit();
                                    }
                                    e.stopPropagation();
                                  }}
                                  onBlur={() => commitEdit()}
                                />
                              ) : (
                                <span className="truncate block">
                                  {cellValue === null
                                    ? 'NULL'
                                    : typeof cellValue === 'object'
                                      ? JSON.stringify(cellValue)
                                      : String(cellValue)}
                                </span>
                              )}
                            </td>
                          );
                        })}
                      </tr>
                    );
                  })}

                  {/* New rows */}
                  {newRows.map((newRow, nri) => (
                    <tr
                      key={`new-${nri}`}
                      className="border-b border-l-2 border-l-emerald-400 bg-emerald-50/50 dark:bg-emerald-950/10"
                    >
                      <td className="px-3 py-1.5 text-center text-xs text-emerald-600 font-medium">
                        +
                      </td>
                      {result.columns.map((col) => (
                        <td key={col.name} className="border-l px-4 py-1.5 font-mono text-xs">
                          <input
                            type="text"
                            className="w-full bg-transparent border-b border-dashed border-emerald-300 dark:border-emerald-700 px-0 py-0.5 text-xs font-mono outline-none select-text"
                            placeholder={col.nullable ? 'NULL' : col.name}
                            value={newRow.values[col.name] === null ? '' : String(newRow.values[col.name])}
                            onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)}
                          />
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </ScrollArea>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {t('common.noData')}
          </div>
        )}
      </div>

      {/* Pagination */}
      <div className="flex shrink-0 items-center justify-between border-t bg-muted/30 px-4 py-1.5">
        <span className="text-xs text-muted-foreground">
          {result ? `${result.rows.length} ${t('query.rows')}` : ''}
          {hasChanges && (
            <span className="ml-2 text-amber-600">
              ({pendingChanges.size} 项修改, {newRows.length} 项新增, {deletedRows.size} 项删除)
            </span>
          )}
          {hasPrimaryKey && (
            <span className="ml-3 text-[10px] opacity-50">Ctrl+C 复制 | Ctrl+V 粘贴 CSV/Excel</span>
          )}
        </span>
        <div className="flex items-center gap-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={page <= 1 || hasChanges}
            onClick={() => setPage((p) => p - 1)}
          >
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <span className="min-w-[60px] text-center text-xs">{t('table.page')} {page}</span>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={!result || result.rows.length < pageSize || hasChanges}
            onClick={() => setPage((p) => p + 1)}
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  );
}
