export type DbType = 'mysql' | 'postgresql' | 'sqlite' | 'sqlserver' | 'mongodb' | 'redis';

export interface ConnectionConfig {
  id: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  username: string;
  password: string;
  database?: string;
  color?: string;
}

export interface ConnectionResult {
  success: boolean;
  message: string;
}

export interface QueryResult {
  columns: ColumnInfo[];
  rows: any[][];
  affected_rows: number;
  execution_time_ms: number;
}

export interface ColumnInfo {
  name: string;
  data_type: string;
  nullable: boolean;
  is_primary_key: boolean;
  default_value?: string;
  comment?: string;
}

export interface TableInfo {
  name: string;
  schema?: string;
  row_count?: number;
  comment?: string;
}

export interface IndexInfo {
  name: string;
  columns: string[];
  is_unique: boolean;
  is_primary: boolean;
}

export interface ViewInfo {
  name: string;
  definition?: string;
}

export interface FunctionInfo {
  name: string;
  language?: string;
  return_type?: string;
  definition?: string;
}

export interface ProcedureInfo {
  name: string;
  language?: string;
  definition?: string;
}

export interface TriggerInfo {
  name: string;
  event: string;
  table: string;
  timing: string;
}

export interface ForeignKeyInfo {
  name: string;
  from_table: string;
  from_columns: string[];
  to_table: string;
  to_columns: string[];
}

export interface UserInfo {
  name: string;
  host?: string;
}

export interface TabItem {
  key: string;
  label: string;
  type: 'query' | 'table-data' | 'table-structure' | 'view-definition' | 'function-definition' | 'procedure-definition' | 'er-diagram' | 'performance' | 'data-chart';
  connectionId: string;
  database: string;
  table?: string;
  sqlContent?: string;
}

export const DB_TYPE_LABELS: Record<DbType, string> = {
  mysql: 'MySQL',
  postgresql: 'PostgreSQL',
  sqlite: 'SQLite',
  sqlserver: 'SQL Server',
  mongodb: 'MongoDB',
  redis: 'Redis',
};

export const DB_TYPE_COLORS: Record<DbType, string> = {
  mysql: '#00758F',
  postgresql: '#336791',
  sqlite: '#003B57',
  sqlserver: '#CC2927',
  mongodb: '#47A248',
  redis: '#DC382D',
};

export const DEFAULT_PORTS: Record<DbType, number> = {
  mysql: 3306,
  postgresql: 5432,
  sqlite: 0,
  sqlserver: 1433,
  mongodb: 27017,
  redis: 6379,
};
