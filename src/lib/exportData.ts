export type ExportFormat = 'csv' | 'json' | 'xlsx';

export interface CsvOptions {
  delimiter: string;
  includeHeader: boolean;
  quoteAll: boolean;
  nullValue: string;
  crlf: boolean;
  bom: boolean;
}

export interface JsonOptions {
  layout: 'objects' | 'arrays';
  pretty: boolean;
}

export interface XlsxOptions {
  includeHeader: boolean;
  sheetName: string;
}

export const FORMAT_EXTENSIONS: Record<ExportFormat, string> = {
  csv: 'csv',
  json: 'json',
  xlsx: 'xlsx',
};

/** Build a default filename like `users_1712345678901.csv` from a base name. */
export function suggestFilename(base: string, format: ExportFormat): string {
  const safe = base.replace(/[^\w-]+/g, '_').replace(/^_+|_+$/g, '') || 'export';
  return `${safe}_${Date.now()}.${FORMAT_EXTENSIONS[format]}`;
}
