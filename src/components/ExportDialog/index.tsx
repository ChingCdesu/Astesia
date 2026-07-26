import { useState, useEffect, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { Input } from '@/components/ui/input';
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from '@/components/ui/select';
import { ColumnInfo } from '@/types/database';
import { notify } from '@/stores/notificationStore';
import { cn } from '@/lib/utils';
import {
  ExportFormat, CsvOptions, JsonOptions, XlsxOptions, suggestFilename, FORMAT_EXTENSIONS,
} from '@/lib/exportData';
import { quoteClickHouseIdentifier } from '@/lib/sqlIdentifier';
import {
  Loader2, FolderOpen, ChevronLeft, ChevronRight, CheckCircle2, FileSpreadsheet,
  FileJson, FileText, Download,
} from 'lucide-react';

/** Source the wizard pulls rows from. */
export type ExportSource =
  | {
      kind: 'table';
      connectionId: string;
      database: string;
      table: string;
      dbType?: string;
      columns: ColumnInfo[];
      appliedWhere: string;
      appliedOrderBy: string;
      totalRows: number | null;
      currentRows: unknown[][];
    }
  | {
      kind: 'static';
      columns: ColumnInfo[];
      rows: unknown[][];
      dbType?: string;
      defaultName: string;
    };

type TableSource = Extract<ExportSource, { kind: 'table' }>;

interface ExportDialogProps {
  open: boolean;
  onClose: () => void;
  source: ExportSource;
}

type Scope = 'current' | 'all' | 'range';
type Phase = 'form' | 'exporting' | 'done';

function quoteIdent(name: string, dbType?: string): string {
  switch (dbType) {
    case 'mysql':
    case 'sqlite':
      return `\`${name.replace(/`/g, '``')}\``;
    case 'clickhouse':
      return quoteClickHouseIdentifier(name);
    case 'sqlserver':
      return `[${name.replace(/]/g, ']]')}]`;
    case 'postgresql':
    default:
      return `"${name.replace(/"/g, '""')}"`;
  }
}

export default function ExportDialog({ open, onClose, source }: ExportDialogProps) {
  const { t } = useTranslation();

  const allColumns = source.columns;
  const maxRows = source.kind === 'table' ? source.totalRows : source.rows.length;
  const currentCount = source.kind === 'table' ? source.currentRows.length : 0;
  const defaultName = source.kind === 'table' ? source.table : source.defaultName;

  const [step, setStep] = useState(0);
  const [scope, setScope] = useState<Scope>(source.kind === 'table' ? 'current' : 'all');
  const [rangeStart, setRangeStart] = useState('1');
  const [rangeEnd, setRangeEnd] = useState(String(maxRows ?? 1000));
  const [selectedCols, setSelectedCols] = useState<Set<string>>(new Set());

  const [format, setFormat] = useState<ExportFormat>('csv');
  const [csv, setCsv] = useState<CsvOptions>({
    delimiter: ',', includeHeader: true, quoteAll: false, nullValue: '', crlf: false, bom: true,
  });
  const [json, setJson] = useState<JsonOptions>({ layout: 'objects', pretty: true });
  const [xlsx, setXlsx] = useState<XlsxOptions>({ includeHeader: true, sheetName: 'Sheet1' });

  const [outputPath, setOutputPath] = useState('');

  const [phase, setPhase] = useState<Phase>('form');
  const [exportedCount, setExportedCount] = useState(0);

  const exporting = phase === 'exporting';

  // Reset everything when the dialog (re)opens.
  useEffect(() => {
    if (!open) return;
    setStep(0);
    setScope(source.kind === 'table' ? 'current' : 'all');
    setRangeStart('1');
    setRangeEnd(String(maxRows ?? 1000));
    setSelectedCols(new Set(allColumns.map((c) => c.name)));
    setFormat('csv');
    setCsv({ delimiter: ',', includeHeader: true, quoteAll: false, nullValue: '', crlf: false, bom: true });
    setJson({ layout: 'objects', pretty: true });
    setXlsx({ includeHeader: true, sheetName: defaultName.slice(0, 31) || 'Sheet1' });
    setOutputPath('');
    setPhase('form');
    setExportedCount(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  // Keep the chosen output path's extension in sync with the format.
  useEffect(() => {
    if (!outputPath) return;
    const ext = FORMAT_EXTENSIONS[format];
    const swapped = outputPath.replace(/\.[^.\\/]+$/, `.${ext}`);
    if (swapped !== outputPath) setOutputPath(swapped);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [format]);

  const selectedColumns = useMemo(
    () => allColumns.filter((c) => selectedCols.has(c.name)),
    [allColumns, selectedCols]
  );
  const selectedIndices = useMemo(
    () => allColumns.map((c, i) => (selectedCols.has(c.name) ? i : -1)).filter((i) => i >= 0),
    [allColumns, selectedCols]
  );

  const rangeStartNum = parseInt(rangeStart, 10);
  const rangeEndNum = parseInt(rangeEnd, 10);
  const rangeValid =
    Number.isInteger(rangeStartNum) &&
    Number.isInteger(rangeEndNum) &&
    rangeStartNum >= 1 &&
    rangeEndNum >= rangeStartNum &&
    (maxRows == null || rangeEndNum <= maxRows);

  const estimatedRows = useMemo(() => {
    if (scope === 'current') return currentCount;
    if (scope === 'range') return rangeValid ? rangeEndNum - rangeStartNum + 1 : 0;
    return maxRows; // 'all' — may be null (unknown)
  }, [scope, currentCount, rangeValid, rangeEndNum, rangeStartNum, maxRows]);

  const step0Valid = selectedCols.size > 0 && (scope !== 'range' || rangeValid);
  const canExport = step0Valid && outputPath.length > 0 && !exporting;

  const toggleColumn = (name: string) => {
    setSelectedCols((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  /** Build a single SQL statement for table-mode "all" / "range" scopes. */
  const buildTableSql = (src: TableSource, all: boolean): string => {
    const qi = (n: string) => quoteIdent(n, src.dbType);
    const tableName = src.table.includes('.')
      ? `${qi(src.table.split('.')[0])}.${qi(src.table.split('.')[1])}`
      : qi(src.table);
    const colList = selectedColumns.map((c) => qi(c.name)).join(', ');
    let sql = `SELECT ${colList} FROM ${tableName}`;
    if (src.appliedWhere) sql += ` WHERE ${src.appliedWhere}`;
    const hasOrder = !!src.appliedOrderBy;
    if (hasOrder) sql += ` ORDER BY ${src.appliedOrderBy}`;
    if (all) return sql;
    const offset = rangeStartNum - 1;
    const limit = rangeEndNum - rangeStartNum + 1;
    if (src.dbType === 'sqlserver') {
      if (!hasOrder) sql += ' ORDER BY (SELECT NULL)';
      sql += ` OFFSET ${offset} ROWS FETCH NEXT ${limit} ROWS ONLY`;
    } else {
      sql += ` LIMIT ${limit} OFFSET ${offset}`;
    }
    return sql;
  };

  const handleExport = async () => {
    if (!canExport) return;
    setPhase('exporting');
    try {
      let exportSource:
        | { kind: 'sql'; sql: string }
        | { kind: 'rows'; columns: string[]; rows: unknown[][] };

      if (source.kind === 'table' && scope !== 'current') {
        exportSource = { kind: 'sql', sql: buildTableSql(source, scope === 'all') };
      } else {
        const baseRows =
          source.kind === 'static'
            ? scope === 'range'
              ? source.rows.slice(rangeStartNum - 1, rangeEndNum)
              : source.rows
            : source.currentRows;
        const projected = baseRows.map((r) => selectedIndices.map((i) => r[i]));
        exportSource = { kind: 'rows', columns: selectedColumns.map((c) => c.name), rows: projected };
      }

      const count = await invoke<number>('export_data', {
        connectionId: source.kind === 'table' ? source.connectionId : '',
        database: source.kind === 'table' ? source.database : '',
        source: exportSource,
        format,
        options: { csv, json, xlsx },
        outputPath,
      });

      setExportedCount(count);
      setPhase('done');
      notify.success(t('export.success'), t('export.successMsg', { count, path: outputPath }));
    } catch (e) {
      setPhase('form');
      notify.error(t('export.failed'), String(e));
    }
  };

  const handleBrowse = async () => {
    const ext = FORMAT_EXTENSIONS[format];
    const path = await save({
      defaultPath: suggestFilename(defaultName, format),
      filters: [{ name: ext.toUpperCase(), extensions: [ext] }],
    });
    if (path) setOutputPath(path);
  };

  const formatMeta: { id: ExportFormat; label: string; icon: typeof FileText }[] = [
    { id: 'csv', label: t('export.formatCsv'), icon: FileText },
    { id: 'json', label: t('export.formatJson'), icon: FileJson },
    { id: 'xlsx', label: t('export.formatExcel'), icon: FileSpreadsheet },
  ];

  const steps = [t('export.stepScope'), t('export.stepFormat'), t('export.stepOutput')];

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v && !exporting) onClose(); }}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Download className="h-4 w-4" />
            {t('export.title')}
          </DialogTitle>
          <DialogDescription>{defaultName}</DialogDescription>
        </DialogHeader>

        {/* Stepper */}
        <div className="flex items-center gap-1">
          {steps.map((label, i) => (
            <div key={i} className="flex flex-1 items-center gap-1">
              <div
                className={cn(
                  'flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs font-medium',
                  i === step
                    ? 'bg-primary text-primary-foreground'
                    : i < step
                      ? 'bg-primary/20 text-primary'
                      : 'bg-muted text-muted-foreground'
                )}
              >
                {i + 1}
              </div>
              <span className={cn('text-xs', i === step ? 'font-medium' : 'text-muted-foreground')}>
                {label}
              </span>
              {i < steps.length - 1 && <div className="mx-1 h-px flex-1 bg-border" />}
            </div>
          ))}
        </div>

        <div className="min-h-[280px]">
          {phase === 'done' ? (
            <div className="flex h-[280px] flex-col items-center justify-center gap-3 text-center">
              <CheckCircle2 className="h-12 w-12 text-green-500" />
              <p className="font-medium">{t('export.success')}</p>
              <p className="text-sm text-muted-foreground">
                {t('export.successMsg', { count: exportedCount, path: outputPath })}
              </p>
            </div>
          ) : exporting ? (
            <div className="flex h-[280px] flex-col items-center justify-center gap-4 px-8 text-center">
              <Loader2 className="h-8 w-8 animate-spin text-primary" />
              <p className="text-sm text-muted-foreground">{t('export.exporting')}</p>
            </div>
          ) : (
            <>
              {/* Step 0 — scope + columns */}
              {step === 0 && (
                <div className="flex flex-col gap-4">
                  <div className="flex flex-col gap-2">
                    <Label>{t('export.scope')}</Label>
                    {source.kind === 'table' && (
                      <ScopeOption
                        active={scope === 'current'}
                        onClick={() => setScope('current')}
                        title={t('export.scopeCurrent')}
                        desc={t('export.scopeCurrentDesc', { count: currentCount })}
                      />
                    )}
                    <ScopeOption
                      active={scope === 'all'}
                      onClick={() => setScope('all')}
                      title={t('export.scopeAll')}
                      desc={
                        maxRows != null
                          ? t('export.scopeAllDesc', { count: maxRows })
                          : t('export.scopeAllUnknown')
                      }
                    />
                    <ScopeOption
                      active={scope === 'range'}
                      onClick={() => setScope('range')}
                      title={t('export.scopeRange')}
                      desc={t('export.scopeRangeDesc')}
                    />
                    {scope === 'range' && (
                      <div className="ml-6 flex items-center gap-2 text-sm">
                        <span className="text-muted-foreground">{t('export.rangeFrom')}</span>
                        <Input
                          type="number"
                          min={1}
                          value={rangeStart}
                          onChange={(e) => setRangeStart(e.target.value)}
                          className="h-8 w-24"
                        />
                        <span className="text-muted-foreground">{t('export.rangeTo')}</span>
                        <Input
                          type="number"
                          min={1}
                          value={rangeEnd}
                          onChange={(e) => setRangeEnd(e.target.value)}
                          className="h-8 w-24"
                        />
                        {!rangeValid && (
                          <span className="text-xs text-destructive">{t('export.invalidRange')}</span>
                        )}
                      </div>
                    )}
                  </div>

                  <div className="flex flex-col gap-2">
                    <div className="flex items-center justify-between">
                      <Label>{t('export.columns')}</Label>
                      <div className="flex gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-6 text-xs"
                          onClick={() => setSelectedCols(new Set(allColumns.map((c) => c.name)))}
                        >
                          {t('export.selectAll')}
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          className="h-6 text-xs"
                          onClick={() => setSelectedCols(new Set())}
                        >
                          {t('export.deselectAll')}
                        </Button>
                      </div>
                    </div>
                    <div className="grid max-h-32 grid-cols-2 gap-x-4 gap-y-1 overflow-y-auto rounded-md border p-2">
                      {allColumns.map((col) => (
                        <label
                          key={col.name}
                          className="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 text-sm hover:bg-accent"
                        >
                          <input
                            type="checkbox"
                            className="h-3.5 w-3.5"
                            checked={selectedCols.has(col.name)}
                            onChange={() => toggleColumn(col.name)}
                          />
                          <span className="truncate" title={col.name}>{col.name}</span>
                        </label>
                      ))}
                    </div>
                  </div>
                </div>
              )}

              {/* Step 1 — format + options */}
              {step === 1 && (
                <div className="flex flex-col gap-4">
                  <div className="flex flex-col gap-2">
                    <Label>{t('export.format')}</Label>
                    <div className="grid grid-cols-3 gap-2">
                      {formatMeta.map(({ id, label, icon: Icon }) => (
                        <button
                          key={id}
                          type="button"
                          onClick={() => setFormat(id)}
                          className={cn(
                            'flex flex-col items-center gap-1.5 rounded-md border p-3 text-sm transition-colors',
                            format === id
                              ? 'border-primary bg-primary/5 text-primary'
                              : 'hover:bg-accent'
                          )}
                        >
                          <Icon className="h-5 w-5" />
                          {label}
                        </button>
                      ))}
                    </div>
                  </div>

                  {format === 'csv' && (
                    <div className="flex flex-col gap-3">
                      <div className="grid grid-cols-2 gap-3">
                        <div className="flex flex-col gap-1.5">
                          <Label className="text-xs">{t('export.csvDelimiter')}</Label>
                          <Select value={csv.delimiter} onValueChange={(v) => setCsv({ ...csv, delimiter: v })}>
                            <SelectTrigger className="h-8">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value=",">{t('export.delimiterComma')}</SelectItem>
                              <SelectItem value=";">{t('export.delimiterSemicolon')}</SelectItem>
                              <SelectItem value={'\t'}>{t('export.delimiterTab')}</SelectItem>
                              <SelectItem value="|">{t('export.delimiterPipe')}</SelectItem>
                            </SelectContent>
                          </Select>
                        </div>
                        <div className="flex flex-col gap-1.5">
                          <Label className="text-xs">{t('export.csvNull')}</Label>
                          <Input
                            value={csv.nullValue}
                            onChange={(e) => setCsv({ ...csv, nullValue: e.target.value })}
                            placeholder="(empty)"
                            className="h-8"
                          />
                        </div>
                      </div>
                      <CheckRow checked={csv.includeHeader} onChange={(v) => setCsv({ ...csv, includeHeader: v })} label={t('export.csvHeader')} />
                      <CheckRow checked={csv.quoteAll} onChange={(v) => setCsv({ ...csv, quoteAll: v })} label={t('export.csvQuoteAll')} />
                      <CheckRow checked={csv.crlf} onChange={(v) => setCsv({ ...csv, crlf: v })} label={t('export.csvCrlf')} />
                      <CheckRow checked={csv.bom} onChange={(v) => setCsv({ ...csv, bom: v })} label={t('export.csvBom')} />
                    </div>
                  )}

                  {format === 'json' && (
                    <div className="flex flex-col gap-3">
                      <div className="flex flex-col gap-1.5">
                        <Label className="text-xs">{t('export.jsonLayout')}</Label>
                        <Select
                          value={json.layout}
                          onValueChange={(v) => setJson({ ...json, layout: v as JsonOptions['layout'] })}
                        >
                          <SelectTrigger className="h-8">
                            <SelectValue />
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value="objects">{t('export.jsonObjects')}</SelectItem>
                            <SelectItem value="arrays">{t('export.jsonArrays')}</SelectItem>
                          </SelectContent>
                        </Select>
                      </div>
                      <CheckRow checked={json.pretty} onChange={(v) => setJson({ ...json, pretty: v })} label={t('export.jsonPretty')} />
                    </div>
                  )}

                  {format === 'xlsx' && (
                    <div className="flex flex-col gap-3">
                      <div className="flex flex-col gap-1.5">
                        <Label className="text-xs">{t('export.xlsxSheet')}</Label>
                        <Input
                          value={xlsx.sheetName}
                          onChange={(e) => setXlsx({ ...xlsx, sheetName: e.target.value })}
                          className="h-8"
                          maxLength={31}
                        />
                      </div>
                      <CheckRow checked={xlsx.includeHeader} onChange={(v) => setXlsx({ ...xlsx, includeHeader: v })} label={t('export.csvHeader')} />
                      <p className="text-xs text-muted-foreground">{t('export.xlsxHint')}</p>
                    </div>
                  )}
                </div>
              )}

              {/* Step 2 — output + summary */}
              {step === 2 && (
                <div className="flex flex-col gap-4">
                  <div className="flex flex-col gap-1.5">
                    <Label>{t('export.outputPath')}</Label>
                    <div className="flex gap-2">
                      <Input readOnly value={outputPath} placeholder={t('export.outputPath')} className="flex-1" />
                      <Button variant="outline" size="sm" onClick={handleBrowse}>
                        <FolderOpen className="mr-1.5 h-3.5 w-3.5" />
                        {t('export.browse')}
                      </Button>
                    </div>
                  </div>

                  <div className="rounded-md border bg-muted/30 p-3 text-sm">
                    <SummaryRow label={t('export.summaryScope')} value={t(`export.scope${scope[0].toUpperCase()}${scope.slice(1)}`)} />
                    <SummaryRow
                      label={t('export.summaryRows')}
                      value={estimatedRows == null ? t('export.scopeAllUnknown') : `${estimatedRows}`}
                    />
                    <SummaryRow label={t('export.summaryColumns')} value={`${selectedCols.size} / ${allColumns.length}`} />
                    <SummaryRow label={t('export.summaryFormat')} value={format.toUpperCase()} />
                  </div>
                </div>
              )}
            </>
          )}
        </div>

        <DialogFooter className="sm:justify-between">
          <div>
            {!exporting && phase !== 'done' && step > 0 && (
              <Button variant="ghost" onClick={() => setStep((s) => s - 1)}>
                <ChevronLeft className="mr-1 h-4 w-4" />
                {t('export.back')}
              </Button>
            )}
          </div>
          <div className="flex gap-2">
            {phase === 'done' ? (
              <Button onClick={onClose}>{t('export.done')}</Button>
            ) : (
              <>
                <Button variant="outline" onClick={onClose} disabled={exporting}>
                  {t('common.cancel')}
                </Button>
                {step < 2 ? (
                  <Button onClick={() => setStep((s) => s + 1)} disabled={step === 0 && !step0Valid}>
                    {t('export.next')}
                    <ChevronRight className="ml-1 h-4 w-4" />
                  </Button>
                ) : (
                  <Button onClick={handleExport} disabled={!canExport}>
                    {exporting && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                    {t('export.start')}
                  </Button>
                )}
              </>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ScopeOption({
  active, onClick, title, desc,
}: { active: boolean; onClick: () => void; title: string; desc: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-start gap-2 rounded-md border p-2.5 text-left transition-colors',
        active ? 'border-primary bg-primary/5' : 'hover:bg-accent'
      )}
    >
      <div
        className={cn(
          'mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center rounded-full border',
          active ? 'border-primary' : 'border-muted-foreground'
        )}
      >
        {active && <div className="h-2 w-2 rounded-full bg-primary" />}
      </div>
      <div className="flex flex-col">
        <span className="text-sm font-medium">{title}</span>
        <span className="text-xs text-muted-foreground">{desc}</span>
      </div>
    </button>
  );
}

function CheckRow({
  checked, onChange, label,
}: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label className="flex cursor-pointer items-center gap-2 text-sm">
      <input
        type="checkbox"
        className="h-3.5 w-3.5"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      {label}
    </label>
  );
}

function SummaryRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between py-0.5">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium">{value}</span>
    </div>
  );
}
