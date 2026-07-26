import { useState, useRef, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Button } from '@/components/ui/button';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { Loader2, Plus, Trash2, ChevronsUpDown, Check } from 'lucide-react';
import { notify } from '@/stores/notificationStore';
import { cn } from '@/lib/utils';
import { quoteClickHouseIdentifier } from '@/lib/sqlIdentifier';

interface Column {
  name: string;
  type: string;
  nullable: boolean;
  primaryKey: boolean;
  defaultValue: string;
}

interface Props {
  connectionId: string;
  database: string;
  schema?: string;
  dbType?: string;
  onSuccess: () => void;
}

const COMMON_TYPES: Record<string, string[]> = {
  postgresql: [
    'SERIAL', 'BIGSERIAL',
    'INTEGER', 'BIGINT', 'SMALLINT',
    'NUMERIC', 'DECIMAL', 'REAL', 'DOUBLE PRECISION',
    'VARCHAR(255)', 'VARCHAR(50)', 'TEXT', 'CHAR(1)',
    'BOOLEAN',
    'DATE', 'TIME', 'TIMESTAMP', 'TIMESTAMPTZ', 'INTERVAL',
    'UUID', 'JSON', 'JSONB',
    'BYTEA', 'INET', 'CIDR', 'MACADDR',
    'ARRAY', 'HSTORE',
  ],
  mysql: [
    'INT', 'BIGINT', 'SMALLINT', 'TINYINT', 'MEDIUMINT',
    'INT AUTO_INCREMENT',
    'DECIMAL(10,2)', 'FLOAT', 'DOUBLE',
    'VARCHAR(255)', 'VARCHAR(50)', 'CHAR(1)', 'TEXT',
    'TINYTEXT', 'MEDIUMTEXT', 'LONGTEXT',
    'BOOLEAN',
    'DATE', 'TIME', 'DATETIME', 'TIMESTAMP', 'YEAR',
    'JSON',
    'BLOB', 'MEDIUMBLOB', 'LONGBLOB',
    'ENUM(...)', 'SET(...)',
    'BINARY(16)',
  ],
  sqlite: [
    'INTEGER', 'INTEGER PRIMARY KEY AUTOINCREMENT',
    'REAL',
    'TEXT',
    'BLOB',
    'NUMERIC',
    'BOOLEAN',
    'DATE', 'DATETIME',
  ],
  sqlserver: [
    'INT', 'BIGINT', 'SMALLINT', 'TINYINT',
    'INT IDENTITY(1,1)',
    'DECIMAL(10,2)', 'FLOAT', 'REAL', 'MONEY', 'SMALLMONEY',
    'NVARCHAR(255)', 'NVARCHAR(50)', 'NVARCHAR(MAX)',
    'VARCHAR(255)', 'VARCHAR(MAX)', 'CHAR(1)',
    'NTEXT', 'TEXT',
    'BIT',
    'DATE', 'TIME', 'DATETIME', 'DATETIME2', 'DATETIMEOFFSET', 'SMALLDATETIME',
    'UNIQUEIDENTIFIER',
    'XML', 'VARBINARY(MAX)', 'IMAGE',
    'GEOGRAPHY', 'GEOMETRY', 'HIERARCHYID',
  ],
  clickhouse: [
    'UInt8', 'UInt16', 'UInt32', 'UInt64', 'UInt128', 'UInt256',
    'Int8', 'Int16', 'Int32', 'Int64', 'Int128', 'Int256',
    'Float32', 'Float64', 'Decimal(18, 4)',
    'String', 'FixedString(16)', 'LowCardinality(String)',
    'Bool', 'UUID', 'IPv4', 'IPv6',
    'Date', 'Date32', 'DateTime', 'DateTime64(3)',
    'Array(String)', 'Tuple(String, UInt64)', 'Map(String, String)',
    "Enum8('unknown' = 0)",
  ],
};

const DEFAULT_TYPES = [
  'INTEGER', 'BIGINT', 'SMALLINT',
  'VARCHAR(255)', 'TEXT', 'CHAR(1)',
  'BOOLEAN',
  'DECIMAL(10,2)', 'FLOAT', 'DOUBLE',
  'DATE', 'TIME', 'TIMESTAMP',
  'JSON', 'BLOB',
];

function getTypesForDb(dbType?: string): string[] {
  if (dbType && COMMON_TYPES[dbType]) return COMMON_TYPES[dbType];
  return DEFAULT_TYPES;
}

const emptyColumn = (dbType?: string): Column => ({
  name: '',
  type: getTypesForDb(dbType)[0] || 'VARCHAR(255)',
  nullable: true,
  primaryKey: false,
  defaultValue: '',
});

function supportsClickHouseNullable(dataType: string): boolean {
  return !/^(Array|Map|Tuple|Nested)\s*\(/i.test(dataType.trim());
}

function clickHouseColumnType(column: Column): string {
  const dataType = column.type.trim();
  if (
    !column.nullable
    || column.primaryKey
    || dataType.startsWith('Nullable(')
    || !supportsClickHouseNullable(dataType)
  ) {
    return dataType;
  }
  if (dataType.startsWith('LowCardinality(') && dataType.endsWith(')')) {
    const inner = dataType.slice('LowCardinality('.length, -1).trim();
    return inner.startsWith('Nullable(')
      ? dataType
      : `LowCardinality(Nullable(${inner}))`;
  }
  return `Nullable(${dataType})`;
}

export default function CreateTableForm({ connectionId, database, schema, dbType, onSuccess }: Props) {
  const { t } = useTranslation();
  const [tableName, setTableName] = useState('');
  const [columns, setColumns] = useState<Column[]>([emptyColumn(dbType)]);
  const [loading, setLoading] = useState(false);

  const types = getTypesForDb(dbType);

  const addColumn = () => setColumns([...columns, emptyColumn(dbType)]);

  const removeColumn = (index: number) => {
    if (columns.length <= 1) return;
    setColumns(columns.filter((_, i) => i !== index));
  };

  const updateColumn = (index: number, field: keyof Column, value: string | boolean) => {
    setColumns(columns.map((col, i) => (i === index ? { ...col, [field]: value } : col)));
  };

  const handleSubmit = async () => {
    if (!tableName.trim() || columns.every((c) => !c.name.trim())) return;
    setLoading(true);
    try {
      const validColumns = columns.filter((c) => c.name.trim());
      const colDefs = validColumns.map((col) => {
        const columnName = dbType === 'clickhouse'
          ? quoteClickHouseIdentifier(col.name)
          : `"${col.name.replace(/"/g, '""')}"`;
        const clickHouseType = dbType === 'clickhouse'
          ? clickHouseColumnType(col)
          : col.type;
        let def = `${columnName} ${clickHouseType}`;
        if (col.primaryKey && dbType !== 'clickhouse') def += ' PRIMARY KEY';
        if (!col.nullable && !col.primaryKey && dbType !== 'clickhouse') def += ' NOT NULL';
        if (col.defaultValue) def += ` DEFAULT ${col.defaultValue}`;
        return def;
      });

      const escapedTableName = tableName.trim().replace(/"/g, '""');
      const qualifiedName = dbType === 'clickhouse'
        ? quoteClickHouseIdentifier(tableName.trim())
        : schema
          ? `"${schema.replace(/"/g, '""')}"."${escapedTableName}"`
          : `"${escapedTableName}"`;
      const primaryKeyColumns = validColumns
        .filter((column) => column.primaryKey)
        .map((column) => (
          dbType === 'clickhouse'
            ? quoteClickHouseIdentifier(column.name)
            : `"${column.name.replace(/"/g, '""')}"`
        ));
      const engine = dbType === 'clickhouse'
        ? `\nENGINE = MergeTree\nORDER BY ${
          primaryKeyColumns.length > 0
            ? `(${primaryKeyColumns.join(', ')})`
            : 'tuple()'
        }`
        : '';
      const sql = `CREATE TABLE ${qualifiedName} (\n  ${colDefs.join(',\n  ')}\n)${engine}`;
      await invoke('execute_query', { connectionId, database, sql });
      notify.success(t('create.success'), `CREATE TABLE ${qualifiedName}`);
      onSuccess();
    } catch (e) {
      notify.error(t('create.failed'), String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div>
        <Label className="mb-1.5 block">{t('create.name')}</Label>
        <Input
          value={tableName}
          onChange={(e) => setTableName(e.target.value)}
          placeholder={t('create.table')}
          autoFocus
        />
      </div>

      <div>
        <div className="flex items-center justify-between mb-2">
          <Label>{t('table.column')}</Label>
          <Button variant="outline" size="sm" onClick={addColumn}>
            <Plus className="h-3.5 w-3.5 mr-1" />
            {t('create.addColumn')}
          </Button>
        </div>

        <div className="space-y-2 max-h-60 overflow-y-auto">
          {columns.map((col, i) => {
            const nullableUnsupported = dbType === 'clickhouse'
              && !supportsClickHouseNullable(col.type);
            return (
              <div key={i} className="flex items-center gap-2 rounded-md border p-2">
                <Input
                  className="flex-1 min-w-0"
                  placeholder={t('create.columnName')}
                  value={col.name}
                  onChange={(e) => updateColumn(i, 'name', e.target.value)}
                />
                <TypeCombobox
                  value={col.type}
                  options={types}
                  onChange={(v) => updateColumn(i, 'type', v)}
                />
                <label className="flex items-center gap-1 text-xs whitespace-nowrap cursor-pointer">
                  <input
                    type="checkbox"
                    className="h-3.5 w-3.5"
                    checked={col.primaryKey}
                    onChange={(e) => updateColumn(i, 'primaryKey', e.target.checked)}
                  />
                  PK
                </label>
                <label className="flex items-center gap-1 text-xs whitespace-nowrap cursor-pointer">
                  <input
                    type="checkbox"
                    className="h-3.5 w-3.5"
                    checked={col.nullable && !nullableUnsupported}
                    onChange={(e) => updateColumn(i, 'nullable', e.target.checked)}
                    disabled={nullableUnsupported}
                  />
                  NULL
                </label>
                <Input
                  className="w-24 shrink-0"
                  placeholder={t('table.defaultValue')}
                  value={col.defaultValue}
                  onChange={(e) => updateColumn(i, 'defaultValue', e.target.value)}
                />
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7 shrink-0"
                  onClick={() => removeColumn(i)}
                  disabled={columns.length <= 1}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            );
          })}
        </div>
      </div>

      <Button onClick={handleSubmit} disabled={loading || !tableName.trim()}>
        {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
        {t('common.confirm')}
      </Button>
    </div>
  );
}

/* Searchable combobox for column type selection */
function TypeCombobox({ value, options, onChange }: { value: string; options: string[]; onChange: (v: string) => void }) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const filtered = search
    ? options.filter((t) => t.toLowerCase().includes(search.toLowerCase()))
    : options;

  useEffect(() => {
    if (open) {
      setSearch('');
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <button
          type="button"
          className={cn(
            "flex h-9 w-44 shrink-0 items-center justify-between rounded-md border border-input bg-transparent px-2 text-xs shadow-sm hover:bg-accent",
            !value && "text-muted-foreground"
          )}
        >
          <span className="truncate">{value || 'Type...'}</span>
          <ChevronsUpDown className="ml-1 h-3.5 w-3.5 shrink-0 opacity-50" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="w-52 p-0" align="start">
        <div className="p-1.5 border-b">
          <input
            ref={inputRef}
            className="w-full rounded-sm border-0 bg-transparent px-2 py-1 text-xs outline-none placeholder:text-muted-foreground select-text"
            placeholder="搜索类型..."
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
        </div>
        <div className="max-h-48 overflow-y-auto p-1">
          {filtered.length === 0 ? (
            <div className="px-2 py-3 text-center text-xs text-muted-foreground">
              无匹配类型，可直接输入
            </div>
          ) : (
            filtered.map((type) => (
              <button
                key={type}
                type="button"
                className={cn(
                  "flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-xs cursor-pointer hover:bg-accent",
                  type === value && "bg-accent"
                )}
                onClick={() => { onChange(type); setOpen(false); }}
              >
                <Check className={cn("h-3 w-3 shrink-0", type === value ? "opacity-100" : "opacity-0")} />
                {type}
              </button>
            ))
          )}
        </div>
        {search && !options.includes(search) && (
          <div className="border-t p-1">
            <button
              type="button"
              className="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-xs cursor-pointer hover:bg-accent text-primary"
              onClick={() => { onChange(search); setOpen(false); }}
            >
              <Plus className="h-3 w-3 shrink-0" />
              使用 "{search}"
            </button>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}
