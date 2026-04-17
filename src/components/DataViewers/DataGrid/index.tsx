import { useState, useEffect, useCallback, useRef, useMemo, memo } from 'react';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { QueryResult, ColumnInfo } from '@/types/database';
import { CellChange, NewRow } from '@/types/editing';
import {
  RefreshCw, Download, Loader2, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight,
  Plus, Trash2, Save, Undo2, AlertTriangle, Maximize2, BarChart3,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { useTabStore } from '@/stores/tabStore';
import ValueViewer from '@/components/DataViewers/DataGrid/ValueViewer';
import { notify } from '@/stores/notificationStore';
import {
  ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger,
} from '@/components/ui/context-menu';
// virtualization removed — 100 rows/page doesn't need it

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
  dbType?: string;
}

/** Quote a SQL identifier based on database type */
function quoteIdent(name: string, dbType?: string): string {
  switch (dbType) {
    case 'mysql':
    case 'sqlite':
      return `\`${name}\``;
    case 'sqlserver':
      return `[${name}]`;
    case 'postgresql':
    default:
      return `"${name}"`;
  }
}

/** Extracted filter bar component — isolated state prevents re-rendering the entire DataGrid */
const FilterBar = memo(function FilterBar({
  onApply,
  onClear,
  hasApplied,
  table,
}: {
  onApply: (where: string, orderBy: string) => void;
  onClear: () => void;
  hasApplied: boolean;
  table: string;
}) {
  const whereRef = useRef<HTMLInputElement>(null);
  const orderByRef = useRef<HTMLInputElement>(null);

  // Reset inputs when table changes
  useEffect(() => {
    if (whereRef.current) whereRef.current.value = '';
    if (orderByRef.current) orderByRef.current.value = '';
  }, [table]);

  const handleApply = useCallback(() => {
    onApply(whereRef.current?.value || '', orderByRef.current?.value || '');
  }, [onApply]);

  const handleClear = useCallback(() => {
    if (whereRef.current) whereRef.current.value = '';
    if (orderByRef.current) orderByRef.current.value = '';
    onClear();
  }, [onClear]);

  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'Enter') handleApply();
  }, [handleApply]);

  return (
    <div className="flex shrink-0 items-center gap-2 border-b bg-muted/20 px-4 py-1">
      <span className="text-[10px] font-medium text-muted-foreground w-14 shrink-0">WHERE</span>
      <input
        ref={whereRef}
        className="flex-1 h-6 rounded border border-input bg-transparent px-2 text-xs font-mono select-text focus:outline-none focus:ring-1 focus:ring-primary"
        placeholder="e.g. age > 18 AND name LIKE '%John%'"
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        onKeyDown={handleKeyDown}
      />
      <span className="text-[10px] font-medium text-muted-foreground w-20 shrink-0">ORDER BY</span>
      <input
        ref={orderByRef}
        className="w-48 h-6 rounded border border-input bg-transparent px-2 text-xs font-mono select-text focus:outline-none focus:ring-1 focus:ring-primary"
        placeholder="e.g. name ASC"
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
        onKeyDown={handleKeyDown}
      />
      <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={handleApply}>
        应用
      </Button>
      {hasApplied && (
        <Button variant="ghost" size="sm" className="h-6 px-2 text-xs" onClick={handleClear}>
          清除
        </Button>
      )}
    </div>
  );
});

interface HistoryEntry {
  type: 'edit' | 'delete' | 'insert';
  key?: string;
  change?: CellChange;
  rowIndices?: number[];
  newRowIndex?: number;
}

export default function DataGrid({ connectionId, database, table, dbType }: Props) {
  const { t } = useTranslation();
  const [result, setResult] = useState<QueryResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize] = useState(100);
  const [totalRows, setTotalRows] = useState<number | null>(null);

  // Editing state
  const [editingCell, setEditingCell] = useState<{ row: number; col: number } | null>(null);
  const [editValue, setEditValue] = useState('');
  const [pendingChanges, setPendingChanges] = useState<Map<string, CellChange>>(new Map());
  const [newRows, setNewRows] = useState<NewRow[]>([]);
  const [deletedRows, setDeletedRows] = useState<Set<number>>(new Set());
  const [selectedRows, setSelectedRows] = useState<Set<number>>(new Set());
  const [changeHistory, setChangeHistory] = useState<HistoryEntry[]>([]);
  const [lastSelectedRow, setLastSelectedRow] = useState<number | null>(null);
  const [enumValues, setEnumValues] = useState<string[]>([]);
  const [editColType, setEditColType] = useState<string>('text');

  // Cell range selection state
  const [selectionStart, setSelectionStart] = useState<{row: number, col: number} | null>(null);
  const [selectionEnd, setSelectionEnd] = useState<{row: number, col: number} | null>(null);
  const [isDragging, setIsDragging] = useState(false);

  // ValueViewer state
  const [valueViewer, setValueViewer] = useState<{
    value: any;
    columnName: string;
    columnType: string;
    rowIndex: number;
    colIndex: number;
  } | null>(null);

  // WHERE / ORDER BY filter state (only "applied" values, input is uncontrolled in FilterBar)
  const [appliedWhere, setAppliedWhere] = useState('');
  const [appliedOrderBy, setAppliedOrderBy] = useState('');

  const handleFilterApply = useCallback((where: string, orderBy: string) => {
    setAppliedWhere(where);
    setAppliedOrderBy(orderBy);
    setPage(1);
  }, []);

  const handleFilterClear = useCallback(() => {
    setAppliedWhere('');
    setAppliedOrderBy('');
    setPage(1);
  }, []);

  const containerRef = useRef<HTMLDivElement>(null);
  const editInputRef = useRef<HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement>(null);

  // Column resize state
  const [columnWidths, setColumnWidths] = useState<Record<string, number>>({});
  const [resizingCol, setResizingCol] = useState<string | null>(null);
  const colResizeRef = useRef({ startX: 0, startWidth: 0, colName: '' });

  // Primary key detection
  const primaryKeyColumn = useMemo(() => {
    if (!result) return null;
    return result.columns.find((c) => c.is_primary_key) ?? null;
  }, [result]);

  const hasPrimaryKey = primaryKeyColumn !== null;

  const hasChanges = pendingChanges.size > 0 || newRows.length > 0 || deletedRows.size > 0;

  const totalRowCount = (result?.rows.length || 0) + newRows.length;

  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const columnMeta = await invoke<ColumnInfo[]>('get_columns', { connectionId, database, table });

      const tableName = table.includes('.')
        ? `${quoteIdent(table.split('.')[0], dbType)}.${quoteIdent(table.split('.')[1], dbType)}`
        : quoteIdent(table, dbType);

      let data: QueryResult;

      if (appliedWhere || appliedOrderBy) {
        // Use execute_query with custom SQL when filters are active
        let sql = `SELECT * FROM ${tableName}`;
        if (appliedWhere) sql += ` WHERE ${appliedWhere}`;
        if (appliedOrderBy) sql += ` ORDER BY ${appliedOrderBy}`;
        // DB-specific pagination
        if (dbType === 'sqlserver') {
          // SQL Server requires ORDER BY for OFFSET/FETCH
          if (!appliedOrderBy) sql += ' ORDER BY (SELECT NULL)';
          sql += ` OFFSET ${(page - 1) * pageSize} ROWS FETCH NEXT ${pageSize} ROWS ONLY`;
        } else {
          sql += ` LIMIT ${pageSize} OFFSET ${(page - 1) * pageSize}`;
        }
        data = await invoke<QueryResult>('execute_query', { connectionId, database, sql });
      } else {
        // Use standard get_table_data (handles DB-specific pagination)
        data = await invoke<QueryResult>('get_table_data', { connectionId, database, table, page, pageSize });
      }

      // If returned empty columns (no rows), use get_columns metadata
      if (data.columns.length === 0 && columnMeta.length > 0) {
        data.columns = columnMeta;
      } else {
        // Merge PK and data_type info from get_columns into result columns
        const metaByName = new Map(columnMeta.map(c => [c.name, c]));
        data.columns = data.columns.map(col => {
          const meta = metaByName.get(col.name);
          return {
            ...col,
            is_primary_key: meta?.is_primary_key ?? col.is_primary_key,
            data_type: meta?.data_type ?? col.data_type,
          };
        });
      }
      setResult(data);

      // Fetch total row count via COUNT query (with WHERE if active)
      try {
        let countSql = `SELECT COUNT(*) FROM ${tableName}`;
        if (appliedWhere) countSql += ` WHERE ${appliedWhere}`;
        const countResult = await invoke<QueryResult>('execute_query', {
          connectionId, database, sql: countSql,
        });
        if (countResult.rows.length > 0 && countResult.rows[0].length > 0) {
          const count = Number(countResult.rows[0][0]);
          if (!isNaN(count)) setTotalRows(count);
        }
      } catch {
        // COUNT failed — leave totalRows as null
      }
    } catch (e: any) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  }, [connectionId, database, table, page, pageSize, appliedWhere, appliedOrderBy, dbType]);

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
    setSelectionStart(null);
    setSelectionEnd(null);
    setIsDragging(false);
    setValueViewer(null);
  }, [connectionId, database, table, page]);

  // Reset filters when table changes (but not on page change)
  useEffect(() => {
    setAppliedWhere('');
    setAppliedOrderBy('');
  }, [connectionId, database, table]);

  // Focus edit input when editing cell changes
  useEffect(() => {
    if (editingCell && editInputRef.current) {
      editInputRef.current.focus();
      if ('select' in editInputRef.current && typeof editInputRef.current.select === 'function') {
        editInputRef.current.select();
      }
    }
  }, [editingCell, editColType]);

  // Cell range selection helper
  const isCellInSelection = useCallback((row: number, col: number) => {
    if (!selectionStart || !selectionEnd) return false;
    const minRow = Math.min(selectionStart.row, selectionEnd.row);
    const maxRow = Math.max(selectionStart.row, selectionEnd.row);
    const minCol = Math.min(selectionStart.col, selectionEnd.col);
    const maxCol = Math.max(selectionStart.col, selectionEnd.col);
    return row >= minRow && row <= maxRow && col >= minCol && col <= maxCol;
  }, [selectionStart, selectionEnd]);

  // Mouse up listener for ending cell drag-select
  useEffect(() => {
    const handleMouseUp = () => setIsDragging(false);
    document.addEventListener('mouseup', handleMouseUp);
    return () => document.removeEventListener('mouseup', handleMouseUp);
  }, []);

  // Keyboard shortcuts handler (Ctrl+Z, Ctrl+V, Ctrl+C)
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Skip custom shortcuts when focus is inside an input/textarea (e.g. filter bar, cell editor)
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;

      if ((e.ctrlKey || e.metaKey) && e.key === 'z') {
        e.preventDefault();
        handleUndo();
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'v') {
        e.preventDefault();
        handlePaste();
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'c') {
        // Cell range selection copy takes priority
        if (selectionStart && selectionEnd && result) {
          e.preventDefault();
          handleCopyCells();
        } else if (selectedRows.size > 0 && result) {
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

  // Column resize handlers
  const handleColumnResizeStart = (e: React.MouseEvent, colName: string) => {
    e.preventDefault();
    e.stopPropagation();
    setResizingCol(colName);
    colResizeRef.current = { startX: e.clientX, startWidth: columnWidths[colName] || 160, colName };
  };

  useEffect(() => {
    if (!resizingCol) return;
    const handleMouseMove = (e: MouseEvent) => {
      const delta = e.clientX - colResizeRef.current.startX;
      const newWidth = Math.max(80, colResizeRef.current.startWidth + delta);
      setColumnWidths(prev => ({ ...prev, [colResizeRef.current.colName]: newWidth }));
    };
    const handleMouseUp = () => setResizingCol(null);
    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
  }, [resizingCol]);

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

  // Determine editor type based on column data_type
  const getEditorType = (dataType: string): 'boolean' | 'enum' | 'datetime' | 'date' | 'time' | 'json' | 'number' | 'text' => {
    const dt = dataType.toLowerCase();
    if (dt === 'boolean' || dt === 'bool' || dt === 'bit' || dt === 'tinyint(1)') return 'boolean';
    if (dt === 'json' || dt === 'jsonb') return 'json';
    if (dt.includes('timestamp') || dt.includes('datetime')) return 'datetime';
    if (dt === 'date') return 'date';
    if (dt === 'time') return 'time';
    if (['int', 'integer', 'bigint', 'smallint', 'tinyint', 'mediumint', 'numeric', 'decimal', 'float', 'double', 'real', 'serial', 'bigserial'].some(t => dt.includes(t))) return 'number';
    return 'text';
  };

  // Known standard SQL text types (used to skip enum lookup)
  const knownTextTypes = ['character varying', 'varchar', 'text', 'char', 'character', 'nvarchar', 'nchar', 'ntext', 'longtext', 'mediumtext', 'tinytext', 'string', 'uuid', 'xml', 'cidr', 'inet', 'macaddr', 'money', 'bytea', 'array'];

  // Move edit focus to an adjacent cell (commits current edit first).
  // Skips deleted rows and stays within existing (non-new) row range.
  const moveEditingCell = (rowDelta: number) => {
    if (!editingCell || !result) return;
    let newRow = editingCell.row + rowDelta;
    while (newRow >= 0 && newRow < result.rows.length && deletedRows.has(newRow)) {
      newRow += rowDelta;
    }
    if (newRow < 0 || newRow >= result.rows.length) return;
    const newCol = editingCell.col;
    commitEdit();
    setTimeout(() => handleCellDoubleClick(newRow, newCol), 0);
  };

  // Render the appropriate cell editor based on column type
  const renderCellEditor = () => {
    const editorKeyDown = (e: React.KeyboardEvent) => {
      if (e.key === 'Enter') { e.preventDefault(); commitEdit(); }
      else if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); moveEditingCell(-1); }
      else if (e.key === 'ArrowDown') { e.preventDefault(); moveEditingCell(1); }
      e.stopPropagation();
    };

    const editorInputClass = "h-full w-full rounded-sm border border-primary/50 bg-background px-2 text-xs select-text focus:outline-none focus:ring-1 focus:ring-primary text-foreground";
    const editorSelectClass = "h-full w-full rounded-sm border border-primary/50 bg-background px-1.5 text-xs select-text focus:outline-none focus:ring-1 focus:ring-primary text-foreground appearance-none cursor-pointer";

    if (editColType === 'boolean') {
      return (
        <select
          ref={editInputRef as React.RefObject<HTMLSelectElement>}
          className={editorSelectClass}
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={commitEdit}
          onKeyDown={editorKeyDown}
        >
          <option value="true">true</option>
          <option value="false">false</option>
          <option value="">NULL</option>
        </select>
      );
    }

    if (editColType === 'enum') {
      return (
        <select
          ref={editInputRef as React.RefObject<HTMLSelectElement>}
          className={editorSelectClass}
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={commitEdit}
          onKeyDown={editorKeyDown}
        >
          <option value="">NULL</option>
          {enumValues.map((v) => (
            <option key={v} value={v}>{v}</option>
          ))}
        </select>
      );
    }

    if (editColType === 'json') {
      return (
        <textarea
          ref={editInputRef as React.RefObject<HTMLTextAreaElement>}
          className="h-20 w-full resize-y rounded-sm border border-primary/50 bg-background px-2 py-1 font-mono text-xs select-text focus:outline-none focus:ring-1 focus:ring-primary text-foreground"
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={commitEdit}
          onKeyDown={(e) => {
            if (e.key === 'Escape') { e.preventDefault(); cancelEdit(); }
            e.stopPropagation();
          }}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
        />
      );
    }

    if (editColType === 'datetime' || editColType === 'date' || editColType === 'time') {
      const inputType = editColType === 'time' ? 'time' : editColType === 'date' ? 'date' : 'datetime-local';
      return (
        <input
          ref={editInputRef as React.RefObject<HTMLInputElement>}
          type={inputType}
          className={editorInputClass}
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={commitEdit}
          onKeyDown={editorKeyDown}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
        />
      );
    }

    if (editColType === 'number') {
      return (
        <input
          ref={editInputRef as React.RefObject<HTMLInputElement>}
          type="number"
          step="any"
          className={editorInputClass}
          value={editValue}
          onChange={(e) => setEditValue(e.target.value)}
          onBlur={commitEdit}
          onKeyDown={editorKeyDown}
          spellCheck={false}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
        />
      );
    }

    // Default: text editor
    return (
      <input
        ref={editInputRef as React.RefObject<HTMLInputElement>}
        type="text"
        className={editorInputClass}
        value={editValue}
        onChange={(e) => setEditValue(e.target.value)}
        onKeyDown={editorKeyDown}
        onBlur={() => commitEdit()}
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        autoCapitalize="off"
      />
    );
  };

  // Cell double-click to start editing
  const handleCellDoubleClick = async (rowIndex: number, colIndex: number) => {
    if (!hasPrimaryKey) return;
    if (deletedRows.has(rowIndex)) return;

    const col = result!.columns[colIndex];
    const key = `${rowIndex}-${col.name}`;
    const existing = pendingChanges.get(key);
    const currentValue = existing ? existing.newValue : result!.rows[rowIndex][colIndex];

    const editorType = getEditorType(col.data_type);
    setEditColType(editorType);
    setEnumValues([]);

    setEditingCell({ row: rowIndex, col: colIndex });

    // Default value for datetime types when current value is null
    if (currentValue === null || currentValue === undefined) {
      if (editorType === 'datetime') {
        setEditValue(new Date().toISOString().slice(0, 16));
      } else if (editorType === 'date') {
        setEditValue(new Date().toISOString().slice(0, 10));
      } else if (editorType === 'time') {
        setEditValue(new Date().toTimeString().slice(0, 8));
      } else {
        setEditValue('');
      }
    } else {
      setEditValue(String(currentValue));
    }

    // If type is 'text' and not a known standard text type, try to fetch enum values
    if (editorType === 'text' && !knownTextTypes.some(t => col.data_type.toLowerCase().includes(t))) {
      try {
        const values = await invoke<string[]>('get_enum_values', {
          connectionId, database, enumType: col.data_type,
        });
        if (values.length > 0) {
          setEditColType('enum');
          setEnumValues(values);
        }
      } catch {
        // Not an enum, keep text editor
      }
    }
  };

  // Commit cell edit — records the change into pendingChanges (saved via "保存更改" button)
  const commitEdit = () => {
    if (!editingCell || !result) {
      setEditingCell(null);
      return;
    }

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
    const bothNull = baseValue === null && newValue === null;
    const sameValue = baseStr === newStr && (bothNull || (baseValue !== null && newValue !== null));

    if (sameValue) {
      // Reverted to original — remove the pending change
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

  // Set a cell value to NULL via context menu — records into pendingChanges
  const handleSetNull = (rowIndex: number, colIndex: number) => {
    if (!result || !hasPrimaryKey) return;
    const column = result.columns[colIndex];
    if (!column.nullable) return;
    const key = `${rowIndex}-${column.name}`;
    const existingChange = pendingChanges.get(key);
    const originalValue = result.rows[rowIndex]?.[colIndex];
    const oldValue = existingChange ? existingChange.oldValue : originalValue;

    if (oldValue === null && !existingChange) return; // Already null, no change

    const change: CellChange = {
      rowIndex,
      columnName: column.name,
      oldValue,
      newValue: null,
    };
    setPendingChanges((map) => {
      const newMap = new Map(map);
      newMap.set(key, change);
      return newMap;
    });
    setChangeHistory((prev) => [...prev, { type: 'edit', key, change }]);
  };

  // Row selection
  const handleRowSelect = useCallback((rowIndex: number, e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }) => {
    if (e.shiftKey && lastSelectedRow !== null) {
      // Shift+click: range select from last selected to current
      const start = Math.min(lastSelectedRow, rowIndex);
      const end = Math.max(lastSelectedRow, rowIndex);
      const rangeSet = new Set<number>();
      for (let i = start; i <= end; i++) rangeSet.add(i);
      setSelectedRows(rangeSet);
    } else if (e.ctrlKey || e.metaKey) {
      // Ctrl+click: toggle this row in the selection
      setSelectedRows((prev) => {
        const newSet = new Set(prev);
        if (newSet.has(rowIndex)) newSet.delete(rowIndex);
        else newSet.add(rowIndex);
        return newSet;
      });
    } else {
      // Plain click: single-select (replace entire selection)
      setSelectedRows(new Set([rowIndex]));
    }
    setLastSelectedRow(rowIndex);
  }, [lastSelectedRow]);

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
      const count = pendingChanges.size + newRows.length + deletedRows.size;
      setPendingChanges(new Map());
      setNewRows([]);
      setDeletedRows(new Set());
      setSelectedRows(new Set());
      setChangeHistory([]);
      setEditingCell(null);
      await loadData();
      notify.success(t('common.success'), `${t('table.saveChanges')}: ${count}`);
    } catch (e: any) {
      console.error('保存失败:', e);
      notify.error(t('common.error'), String(e));
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

      // If there's a cell selection start point, paste into existing cells starting from there
      if (selectionStart) {
        const startRow = selectionStart.row;
        const startCol = selectionStart.col;

        parsedRows.forEach((row, rowOffset) => {
          const targetRow = startRow + rowOffset;
          if (targetRow >= result.rows.length) return; // Don't paste beyond existing rows

          row.forEach((cell, colOffset) => {
            const targetCol = startCol + colOffset;
            if (targetCol >= result.columns.length) return;

            const column = result.columns[targetCol];
            const key = `${targetRow}-${column.name}`;
            const oldValue = result.rows[targetRow][targetCol];
            const newValue = cell === '' || cell === 'NULL' ? null : cell;

            const change: CellChange = { rowIndex: targetRow, columnName: column.name, oldValue, newValue };
            setPendingChanges(prev => { const m = new Map(prev); m.set(key, change); return m; });
            setChangeHistory(prev => [...prev, { type: 'edit', key, change }]);
          });
        });
        return;
      }

      // No selection — add as new rows (existing behavior)
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

  // Get display value for a cell (considering pending changes)
  const getCellDisplayValue = useCallback((rowIndex: number, colIndex: number): any => {
    if (!result) return null;
    const col = result.columns[colIndex];
    const key = `${rowIndex}-${col.name}`;
    const change = pendingChanges.get(key);
    return change ? change.newValue : result.rows[rowIndex][colIndex];
  }, [result, pendingChanges]);

  // Copy selected rows as TSV (for Excel compatibility)
  const handleCopyRows = useCallback(() => {
    if (!result || selectedRows.size === 0) return;
    const headers = result.columns.map(c => c.name).join('\t');
    const rows = Array.from(selectedRows)
      .sort((a, b) => a - b)
      .map(rowIdx =>
        result.columns.map((_, colIdx) => {
          const val = getCellDisplayValue(rowIdx, colIdx);
          if (val === null) return '';
          if (typeof val === 'object') return JSON.stringify(val);
          return String(val);
        }).join('\t')
      )
      .join('\n');
    navigator.clipboard.writeText(headers + '\n' + rows);
  }, [result, selectedRows, getCellDisplayValue]);

  // Copy selected cell range as TSV
  const handleCopyCells = useCallback(() => {
    if (!result || !selectionStart || !selectionEnd) return;
    const minRow = Math.min(selectionStart.row, selectionEnd.row);
    const maxRow = Math.max(selectionStart.row, selectionEnd.row);
    const minCol = Math.min(selectionStart.col, selectionEnd.col);
    const maxCol = Math.max(selectionStart.col, selectionEnd.col);
    const headers = result.columns.slice(minCol, maxCol + 1).map(c => c.name).join('\t');
    const rows: string[] = [];
    for (let r = minRow; r <= maxRow; r++) {
      const cells: string[] = [];
      for (let c = minCol; c <= maxCol; c++) {
        const val = getCellDisplayValue(r, c);
        if (val === null) cells.push('');
        else if (typeof val === 'object') cells.push(JSON.stringify(val));
        else cells.push(String(val));
      }
      rows.push(cells.join('\t'));
    }
    navigator.clipboard.writeText(headers + '\n' + rows.join('\n'));
  }, [result, selectionStart, selectionEnd, getCellDisplayValue]);

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
    <div ref={containerRef} className="flex h-full flex-col" tabIndex={-1} onContextMenu={(e) => e.preventDefault()}>
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

        <div className="mx-1 h-4 w-px bg-border" />
        <Button variant="ghost" size="sm" onClick={() => {
          useTabStore.getState().addTab({
            key: `chart-${connectionId}-${database}-${table}`,
            label: `${table.split('.').pop()} [图表] [${database}]`,
            type: 'data-chart' as any,
            connectionId,
            database,
            table,
          });
        }}>
          <BarChart3 className="mr-1.5 h-3.5 w-3.5" />
          图表
        </Button>

        <div className="ml-auto flex items-center gap-1.5">
          {(() => {
            const tableParts = table.includes('.') ? table.split('.') : [table];
            return (
              <>
                <Badge variant="outline" className="font-mono text-[11px]">{database}</Badge>
                {tableParts.length > 1 && (
                  <>
                    <span className="text-muted-foreground">.</span>
                    <Badge variant="outline" className="font-mono text-[11px]">{tableParts[0]}</Badge>
                  </>
                )}
                <span className="text-muted-foreground">.</span>
                <Badge variant="outline" className="font-mono text-[11px]">{tableParts[tableParts.length - 1]}</Badge>
              </>
            );
          })()}
        </div>
      </div>

      {/* WHERE / ORDER BY filter bar */}
      <FilterBar
        table={table}
        hasApplied={!!(appliedWhere || appliedOrderBy)}
        onApply={handleFilterApply}
        onClear={handleFilterClear}
      />

      {/* Table */}
      <div className="flex-1 overflow-hidden">
        {loading && !result ? (
          <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
            <Loader2 className="h-5 w-5 animate-spin" />
            <span>{t('common.loading')}</span>
          </div>
        ) : result && result.columns.length > 0 ? (
          <div className="flex-1 overflow-auto h-full" onContextMenu={(e) => e.preventDefault()}>
              <table className="text-sm" style={{ tableLayout: 'fixed', width: 'max-content', minWidth: '100%' }}>
                <thead className="sticky top-0 z-10 bg-muted/60">
                  <tr className="border-b bg-muted/60">
                    <th className="w-12 px-3 py-2 text-center text-xs font-medium text-muted-foreground">#</th>
                    {result.columns.map((col) => (
                      <th
                        key={col.name}
                        style={{ width: columnWidths[col.name] || 160, minWidth: 80 }}
                        className="relative whitespace-nowrap border-l px-4 py-2 text-left text-xs font-medium"
                      >
                        <div className="flex items-center gap-1.5">
                          {col.is_primary_key && <Badge variant="warning" className="px-1 py-0 text-[9px]">PK</Badge>}
                          <span>{col.name}</span>
                        </div>
                        <span className="mt-0.5 block text-[10px] font-normal text-muted-foreground">{col.data_type}</span>
                        <div
                          className="absolute right-0 top-0 h-full w-1 cursor-col-resize hover:bg-primary/30"
                          onMouseDown={(e) => handleColumnResizeStart(e, col.name)}
                        />
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {/* Data rows */}
                  {totalRowCount > 0 ? (
                    Array.from({ length: totalRowCount }, (_, ri) => {
                      const existingRowCount = result.rows.length;
                      const isNewRow = ri >= existingRowCount;

                      if (isNewRow) {
                        // Render new row
                        const nri = ri - existingRowCount;
                        const newRow = newRows[nri];
                        if (!newRow) return null;

                        return (
                          <tr
                            key={`new-${nri}`}
                            data-index={ri}
                            className="border-b border-l-2 border-l-emerald-400 bg-emerald-50/50 dark:bg-emerald-950/10"
                          >
                            <td className="px-3 py-1.5 text-center text-xs text-emerald-600 font-medium">
                              +
                            </td>
                            {result.columns.map((col) => (
                              <td
                                key={col.name}
                                style={{ width: columnWidths[col.name] || 160, minWidth: 80 }}
                                className="border-l px-4 py-1.5 font-mono text-xs"
                              >
                                {(() => {
                                  const dt = col.data_type.toLowerCase();
                                  const baseClass = "w-full rounded-sm border border-dashed border-emerald-400/60 bg-background px-1.5 py-0.5 text-xs font-mono text-foreground outline-none select-text focus:border-solid focus:border-primary/50 focus:ring-1 focus:ring-primary";
                                  const val = newRow.values[col.name] === null ? '' : String(newRow.values[col.name]);

                                  if (dt === 'boolean' || dt === 'bool') {
                                    return (
                                      <select className={baseClass + " cursor-pointer"} value={val} onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)}>
                                        <option value="">NULL</option>
                                        <option value="true">true</option>
                                        <option value="false">false</option>
                                      </select>
                                    );
                                  }
                                  if (dt.includes('timestamp') || dt.includes('datetime')) {
                                    return <input type="datetime-local" className={baseClass} value={val || new Date().toISOString().slice(0, 16)} onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)} spellCheck={false} autoComplete="off" autoCorrect="off" autoCapitalize="off" />;
                                  }
                                  if (dt === 'date') {
                                    return <input type="date" className={baseClass} value={val || new Date().toISOString().slice(0, 10)} onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)} spellCheck={false} autoComplete="off" autoCorrect="off" autoCapitalize="off" />;
                                  }
                                  if (dt === 'time' || dt.includes('time without') || dt.includes('timetz')) {
                                    return <input type="time" className={baseClass} value={val || new Date().toTimeString().slice(0, 8)} onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)} spellCheck={false} autoComplete="off" autoCorrect="off" autoCapitalize="off" />;
                                  }
                                  if (dt === 'json' || dt === 'jsonb') {
                                    return <textarea className={baseClass + " h-12 resize-y"} placeholder="{}" value={val} onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)} spellCheck={false} autoComplete="off" autoCorrect="off" autoCapitalize="off" />;
                                  }
                                  const isNum = ['int', 'integer', 'bigint', 'smallint', 'numeric', 'decimal', 'float', 'double', 'real', 'serial'].some(t => dt.includes(t));
                                  return (
                                    <input
                                      type={isNum ? 'number' : 'text'}
                                      step={isNum ? (dt.includes('int') ? '1' : 'any') : undefined}
                                      className={baseClass}
                                      placeholder={col.nullable ? 'NULL' : col.name}
                                      value={val}
                                      onChange={(e) => handleNewRowCellChange(nri, col.name, e.target.value)}
                                      spellCheck={false}
                                      autoComplete="off"
                                      autoCorrect="off"
                                      autoCapitalize="off"
                                    />
                                  );
                                })()}
                              </td>
                            ))}
                          </tr>
                        );
                      }

                      // Render existing row
                      const isDeleted = deletedRows.has(ri);
                      const isSelected = selectedRows.has(ri);

                      return (
                        <tr
                          key={ri}
                          data-index={ri}
                          onClick={(e) => handleRowSelect(ri, e)}
                          className={cn(
                            "border-b transition-colors cursor-pointer",
                            isDeleted
                              ? "bg-red-50 line-through dark:bg-red-950/20"
                              : isSelected
                                ? "bg-blue-500/10 dark:bg-blue-400/10"
                                : "hover:bg-muted/30",
                          )}
                        >
                          <td
                            className={cn(
                              "px-3 py-1.5 text-center text-xs text-muted-foreground cursor-pointer select-none",
                              isSelected && "font-bold text-blue-600 dark:text-blue-400",
                            )}
                            onClick={(e) => handleRowSelect(ri, e)}
                          >
                            {(page - 1) * pageSize + ri + 1}
                          </td>
                          {result.rows[ri].map((_, ci) => {
                            const cellValue = getCellDisplayValue(ri, ci);
                            const dirty = isCellDirty(ri, ci);
                            const isEditing = editingCell?.row === ri && editingCell?.col === ci;
                            const inCellSelection = isCellInSelection(ri, ci);

                            return (
                              <ContextMenu key={ci}>
                                <ContextMenuTrigger asChild>
                                  <td
                                    style={{ width: columnWidths[result.columns[ci].name] || 160, minWidth: 80 }}
                                    className={cn(
                                      "border-l px-4 py-1.5 font-mono text-xs overflow-hidden",
                                      cellValue === null && !isEditing && "italic text-muted-foreground/50",
                                      typeof cellValue === 'object' && cellValue !== null && "text-blue-600",
                                      dirty && "bg-amber-50 dark:bg-amber-950/20 border-l-2 border-l-amber-400",
                                      isDeleted && "opacity-50",
                                      inCellSelection && !dirty && "bg-blue-100 dark:bg-blue-900/30",
                                    )}
                                    onDoubleClick={() => handleCellDoubleClick(ri, ci)}
                                    onMouseDown={(e) => {
                                      if (e.detail === 2) return; // double-click handled separately
                                      if (isEditing) return; // allow caret positioning inside active editor
                                      setSelectionStart({ row: ri, col: ci });
                                      setSelectionEnd({ row: ri, col: ci });
                                      setIsDragging(true);
                                      e.preventDefault();
                                    }}
                                    onMouseEnter={() => {
                                      if (isDragging) {
                                        setSelectionEnd({ row: ri, col: ci });
                                      }
                                    }}
                                  >
                                    {isEditing ? (
                                      renderCellEditor()
                                    ) : cellValue === null ? (
                                      <span className="truncate block">NULL</span>
                                    ) : typeof cellValue === 'object' && cellValue !== null ? (
                                      <span className="flex items-center gap-1">
                                        <span className="truncate text-blue-600 dark:text-blue-400">
                                          {JSON.stringify(cellValue).slice(0, 50)}{JSON.stringify(cellValue).length > 50 ? '...' : ''}
                                        </span>
                                        <button className="shrink-0 rounded p-0.5 hover:bg-muted" onClick={(e) => {
                                          e.stopPropagation();
                                          setValueViewer({ value: cellValue, columnName: result.columns[ci].name, columnType: result.columns[ci].data_type, rowIndex: ri, colIndex: ci });
                                        }}>
                                          <Maximize2 className="h-3 w-3 text-muted-foreground" />
                                        </button>
                                      </span>
                                    ) : typeof cellValue === 'string' && cellValue.length > 100 ? (
                                      <span className="flex items-center gap-1">
                                        <span className="truncate">{cellValue.slice(0, 100)}...</span>
                                        <button className="shrink-0 rounded p-0.5 hover:bg-muted" onClick={(e) => {
                                          e.stopPropagation();
                                          setValueViewer({ value: cellValue, columnName: result.columns[ci].name, columnType: result.columns[ci].data_type, rowIndex: ri, colIndex: ci });
                                        }}>
                                          <Maximize2 className="h-3 w-3 text-muted-foreground" />
                                        </button>
                                      </span>
                                    ) : (
                                      <span className="truncate block">{String(cellValue)}</span>
                                    )}
                                  </td>
                                </ContextMenuTrigger>
                                <ContextMenuContent>
                                  <ContextMenuItem
                                    disabled={!hasPrimaryKey || !result.columns[ci].nullable || isDeleted}
                                    onClick={() => handleSetNull(ri, ci)}
                                  >
                                    设置为 NULL
                                  </ContextMenuItem>
                                  <ContextMenuSeparator />
                                  <ContextMenuItem
                                    disabled={!hasPrimaryKey || isDeleted}
                                    onClick={() => handleCellDoubleClick(ri, ci)}
                                  >
                                    编辑
                                  </ContextMenuItem>
                                  <ContextMenuItem
                                    onClick={() => {
                                      const v = cellValue === null ? '' : typeof cellValue === 'object' ? JSON.stringify(cellValue) : String(cellValue);
                                      navigator.clipboard.writeText(v);
                                    }}
                                  >
                                    复制值
                                  </ContextMenuItem>
                                </ContextMenuContent>
                              </ContextMenu>
                            );
                          })}
                        </tr>
                      );
                    })
                  ) : (
                    <tr>
                      <td colSpan={result.columns.length + 1} className="py-12 text-center text-sm text-muted-foreground">
                        {t('common.noData')}
                      </td>
                    </tr>
                  )}

                </tbody>
              </table>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
            {t('common.noData')}
          </div>
        )}
      </div>

      {/* Pagination */}
      <div className="flex shrink-0 items-center justify-between border-t bg-muted/30 px-4 py-1.5">
        <span className="text-xs text-muted-foreground">
          {result ? (
            <>
              显示 {result.rows.length > 0 ? (page-1)*pageSize + 1 : 0} - {(page-1)*pageSize + result.rows.length} 行
              {totalRows !== null && <span className="ml-1">/ 共 {totalRows} 行</span>}
            </>
          ) : ''}
          {hasChanges && (
            <span className="ml-2 text-amber-600">
              ({pendingChanges.size} 项修改, {newRows.length} 项新增, {deletedRows.size} 项删除)
            </span>
          )}
          {hasPrimaryKey && (
            <span className="ml-2 text-[10px] opacity-50">Ctrl+C 复制 | Ctrl+V 粘贴</span>
          )}
        </span>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={page <= 1 || hasChanges}
            onClick={() => setPage(1)}
          >
            <ChevronsLeft className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={page <= 1 || hasChanges}
            onClick={() => setPage((p) => p - 1)}
          >
            <ChevronLeft className="h-4 w-4" />
          </Button>
          <input
            type="number"
            min={1}
            value={page}
            onChange={(e) => {
              const p = parseInt(e.target.value);
              if (p > 0 && !hasChanges) setPage(p);
            }}
            className="h-7 w-14 rounded border border-input bg-transparent px-2 text-center text-xs select-text"
            disabled={hasChanges}
          />
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={!result || result.rows.length < pageSize || hasChanges}
            onClick={() => setPage((p) => p + 1)}
          >
            <ChevronRight className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            disabled={!result || result.rows.length < pageSize || hasChanges || totalRows === null}
            onClick={() => {
              if (totalRows !== null) setPage(Math.max(1, Math.ceil(totalRows / pageSize)));
            }}
          >
            <ChevronsRight className="h-4 w-4" />
          </Button>
          <span className="ml-2 text-xs text-muted-foreground">{pageSize} 行/页</span>
        </div>
      </div>

      {/* ValueViewer dialog for JSON/large text */}
      {valueViewer && (
        <ValueViewer
          open={!!valueViewer}
          onClose={() => setValueViewer(null)}
          value={valueViewer.value}
          columnName={valueViewer.columnName}
          columnType={valueViewer.columnType}
          readOnly={!hasPrimaryKey}
          onSave={(newValue) => {
            const key = `${valueViewer.rowIndex}-${valueViewer.columnName}`;
            const change: CellChange = {
              rowIndex: valueViewer.rowIndex,
              columnName: valueViewer.columnName,
              oldValue: result!.rows[valueViewer.rowIndex][valueViewer.colIndex],
              newValue,
            };
            setPendingChanges(prev => { const m = new Map(prev); m.set(key, change); return m; });
            setChangeHistory(prev => [...prev, { type: 'edit', key, change }]);
            setValueViewer(null);
          }}
        />
      )}
    </div>
  );
}
