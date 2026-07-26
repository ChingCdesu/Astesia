export type DbType =
  | 'mysql'
  | 'postgresql'
  | 'sqlite'
  | 'sqlserver'
  | 'mongodb'
  | 'redis'
  | 'clickhouse';

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
  group_name?: string;
  tags?: string[];
  source?: 'shared';
  has_credential?: boolean;
  revision?: number;
  mcp_enabled?: boolean;
  mcp_in_use?: boolean;
  mcp_connected?: boolean;
  mcp_session_count?: number;
  disconnecting?: boolean;
  last_error?: string | null;
}

export interface SharedConnectionProfile {
  id: string;
  name: string;
  db_type: DbType;
  host: string;
  port: number;
  username: string;
  database?: string;
  color?: string;
  group_name?: string;
  tags: string[];
  has_credential: boolean;
  revision: number;
  mcp_enabled: boolean;
}

export interface ConnectionProfilesSnapshot {
  revision: number;
  profiles: SharedConnectionProfile[];
}

export interface ConnectionRepositoryError {
  code: string;
  message: string;
  remediation: string;
  retryable: boolean;
  details?: Record<string, unknown>;
}

export interface LegacyMigrationResult {
  imported: number;
  skipped: number;
  already_complete: boolean;
  revision: number;
}

export interface McpConnectionUsage {
  id: string;
  profile_revision: number;
  mcp_in_use: boolean;
  mcp_connected: boolean;
  mcp_session_count: number;
  disconnecting: boolean;
  last_error?: string | null;
}

export interface McpConnectionsSnapshot {
  revision: number;
  connections: McpConnectionUsage[];
}

export interface ConnectionResult {
  success: boolean;
  message: string;
}

export interface DisconnectConnectionResult extends ConnectionResult {
  partial: boolean;
  error_code?: string;
  external_mcp_in_use: boolean;
}

export interface QueryResult {
  columns: ColumnInfo[];
  rows: unknown[][];
  affected_rows: number;
  execution_time_ms: number;
}

export interface StatementResult {
  sql: string;
  success: boolean;
  error?: string | null;
  columns: ColumnInfo[];
  rows: unknown[][];
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
  type: 'query' | 'table-data' | 'table-structure' | 'view-definition' | 'function-definition' | 'procedure-definition' | 'er-diagram' | 'performance' | 'data-chart' | 'redis-viewer' | 'mongo-viewer';
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
  clickhouse: 'ClickHouse',
};

export const DB_TYPE_COLORS: Record<DbType, string> = {
  mysql: '#00758F',
  postgresql: '#336791',
  sqlite: '#003B57',
  sqlserver: '#CC2927',
  mongodb: '#47A248',
  redis: '#DC382D',
  clickhouse: '#FFCC01',
};

export const DEFAULT_PORTS: Record<DbType, number> = {
  mysql: 3306,
  postgresql: 5432,
  sqlite: 0,
  sqlserver: 1433,
  mongodb: 27017,
  redis: 6379,
  clickhouse: 8123,
};
