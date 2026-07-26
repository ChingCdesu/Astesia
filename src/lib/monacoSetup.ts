import type { languages } from 'monaco-editor';
import { quoteClickHouseIdentifier } from './sqlIdentifier';

// --- Common SQL keywords (all dialects) ---
const COMMON_SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'AND', 'OR', 'NOT', 'NULL', 'IS', 'IN', 'LIKE', 'BETWEEN',
  'ORDER', 'BY', 'GROUP', 'HAVING', 'LIMIT', 'OFFSET', 'AS', 'DISTINCT', 'ALL', 'ANY', 'SOME',
  'JOIN', 'INNER', 'LEFT', 'RIGHT', 'FULL', 'OUTER', 'ON', 'USING', 'CROSS',
  'UNION', 'INTERSECT', 'EXCEPT', 'WITH', 'RECURSIVE',
  'INSERT', 'INTO', 'VALUES', 'UPDATE', 'SET', 'DELETE',
  'CREATE', 'TABLE', 'VIEW', 'INDEX', 'DATABASE', 'SCHEMA', 'IF', 'EXISTS',
  'ALTER', 'ADD', 'DROP', 'COLUMN', 'CONSTRAINT', 'PRIMARY', 'KEY', 'FOREIGN', 'REFERENCES',
  'UNIQUE', 'CHECK', 'DEFAULT', 'AUTO_INCREMENT', 'IDENTITY',
  'BEGIN', 'COMMIT', 'ROLLBACK', 'TRANSACTION', 'SAVEPOINT', 'START',
  'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'IIF',
  'TRUE', 'FALSE', 'CAST', 'CONVERT',
  'INT', 'INTEGER', 'BIGINT', 'SMALLINT', 'TINYINT', 'DECIMAL', 'NUMERIC', 'FLOAT', 'DOUBLE', 'REAL',
  'CHAR', 'VARCHAR', 'TEXT', 'BLOB', 'DATE', 'TIME', 'DATETIME', 'TIMESTAMP', 'BOOLEAN',
  'ASC', 'DESC',
  'EXPLAIN', 'ANALYZE',
];

// --- Common SQL functions (all dialects) ---
const COMMON_SQL_FUNCTIONS = [
  // Aggregate
  'COUNT', 'SUM', 'AVG', 'MIN', 'MAX',
  // String
  'CONCAT', 'SUBSTRING', 'SUBSTR', 'LENGTH', 'LOWER', 'UPPER', 'TRIM', 'LTRIM', 'RTRIM',
  'REPLACE', 'POSITION', 'INSTR', 'REVERSE', 'LEFT', 'RIGHT', 'REPEAT',
  // Math
  'ABS', 'CEIL', 'CEILING', 'FLOOR', 'ROUND', 'TRUNC', 'POWER', 'SQRT', 'MOD', 'RANDOM', 'RAND',
  'GREATEST', 'LEAST', 'SIGN', 'EXP', 'LOG', 'LN',
  // Date/Time
  'NOW', 'CURRENT_DATE', 'CURRENT_TIME', 'CURRENT_TIMESTAMP', 'EXTRACT',
  // Conditional
  'COALESCE', 'NULLIF', 'IFNULL', 'ISNULL',
];

// --- Dialect-specific keywords ---
const MYSQL_KEYWORDS = [
  'SHOW', 'DATABASES', 'TABLES', 'DESCRIBE', 'USE', 'ENGINE', 'CHARSET', 'COLLATE',
  'REGEXP', 'BINARY', 'UNSIGNED', 'ZEROFILL', 'ENUM', 'MEDIUMINT',
  'TINYTEXT', 'MEDIUMTEXT', 'LONGTEXT', 'TINYBLOB', 'MEDIUMBLOB', 'LONGBLOB', 'JSON',
  'YEAR', 'STRAIGHT_JOIN', 'LOCK', 'UNLOCK', 'OPTIMIZE',
];

const MYSQL_FUNCTIONS = [
  'NOW', 'CURDATE', 'CURTIME', 'DATE_FORMAT', 'STR_TO_DATE', 'DATE_ADD', 'DATE_SUB', 'DATEDIFF',
  'TIMESTAMPDIFF', 'TIMESTAMPADD', 'UNIX_TIMESTAMP', 'FROM_UNIXTIME', 'YEAR', 'MONTH', 'DAY',
  'HOUR', 'MINUTE', 'SECOND', 'DAYNAME', 'MONTHNAME', 'WEEK', 'WEEKDAY', 'QUARTER',
  'CONCAT_WS', 'GROUP_CONCAT', 'FIELD', 'FIND_IN_SET', 'LOCATE', 'CHAR_LENGTH', 'CHARACTER_LENGTH',
  'LPAD', 'RPAD', 'SPACE', 'LCASE', 'UCASE', 'INSERT', 'ELT', 'EXPORT_SET', 'MAKE_SET', 'OCT',
  'IF', 'IFNULL', 'NULLIF', 'CASE',
  'JSON_OBJECT', 'JSON_ARRAY', 'JSON_EXTRACT', 'JSON_KEYS', 'JSON_LENGTH', 'JSON_VALID',
  'JSON_CONTAINS', 'JSON_SET', 'JSON_INSERT', 'JSON_REPLACE', 'JSON_REMOVE', 'JSON_MERGE',
  'LAST_INSERT_ID', 'ROW_COUNT', 'CONNECTION_ID', 'DATABASE', 'USER', 'VERSION',
  'MD5', 'SHA1', 'SHA2', 'PASSWORD', 'AES_ENCRYPT', 'AES_DECRYPT', 'UUID',
];

const POSTGRES_KEYWORDS = [
  'RETURNING', 'ILIKE', 'SIMILAR', 'ARRAY', 'JSONB', 'HSTORE', 'SERIAL', 'BIGSERIAL', 'SMALLSERIAL',
  'BYTEA', 'UUID', 'INET', 'CIDR', 'MACADDR', 'MONEY', 'INTERVAL', 'TSQUERY', 'TSVECTOR', 'REGCLASS',
  'LATERAL', 'MATERIALIZED', 'REFRESH', 'EXTENSION', 'CONCURRENTLY', 'VACUUM',
  'REINDEX', 'CLUSTER', 'NOTIFY', 'LISTEN', 'UNLISTEN', 'COPY', 'EXCLUDE', 'PARTITION',
  'INHERIT', 'RULE', 'WINDOW', 'OVER', 'PARTITION', 'OVERLAPS',
];

const POSTGRES_FUNCTIONS = [
  'NOW', 'CURRENT_DATE', 'CURRENT_TIME', 'CURRENT_TIMESTAMP', 'AGE', 'DATE_PART', 'DATE_TRUNC',
  'TO_CHAR', 'TO_DATE', 'TO_TIMESTAMP', 'TO_NUMBER', 'EXTRACT', 'JUSTIFY_DAYS', 'JUSTIFY_HOURS',
  'CONCAT', 'CONCAT_WS', 'STRING_AGG', 'ARRAY_AGG', 'ARRAY_LENGTH', 'ARRAY_APPEND', 'ARRAY_PREPEND',
  'ARRAY_CAT', 'ARRAY_REMOVE', 'ARRAY_REPLACE', 'UNNEST', 'GENERATE_SERIES',
  'JSONB_BUILD_OBJECT', 'JSONB_BUILD_ARRAY', 'JSONB_AGG', 'JSONB_OBJECT_AGG', 'JSONB_ARRAY_ELEMENTS',
  'JSONB_ARRAY_ELEMENTS_TEXT', 'JSONB_OBJECT_KEYS', 'JSONB_PATH_EXISTS', 'JSONB_PATH_QUERY',
  'JSON_BUILD_OBJECT', 'JSON_BUILD_ARRAY', 'JSON_AGG',
  'COALESCE', 'NULLIF', 'GREATEST', 'LEAST',
  'LENGTH', 'CHAR_LENGTH', 'OCTET_LENGTH', 'SUBSTRING', 'LOWER', 'UPPER', 'INITCAP',
  'LPAD', 'RPAD', 'TRIM', 'LTRIM', 'RTRIM', 'REPLACE', 'TRANSLATE', 'OVERLAY', 'POSITION',
  'REGEXP_MATCH', 'REGEXP_MATCHES', 'REGEXP_REPLACE', 'REGEXP_SPLIT_TO_ARRAY', 'REGEXP_SPLIT_TO_TABLE',
  'GEN_RANDOM_UUID', 'UUID_GENERATE_V4', 'MD5', 'CRYPT',
  'ROW_NUMBER', 'RANK', 'DENSE_RANK', 'LAG', 'LEAD', 'FIRST_VALUE', 'LAST_VALUE', 'NTILE',
  'PG_TYPEOF', 'PG_TABLE_SIZE', 'PG_DATABASE_SIZE', 'CURRENT_USER', 'SESSION_USER', 'CURRENT_DATABASE',
];

const SQLITE_KEYWORDS = [
  'PRAGMA', 'AUTOINCREMENT', 'GLOB', 'VACUUM', 'ATTACH', 'DETACH', 'REINDEX', 'INDEXED',
  'CONFLICT', 'ABORT', 'FAIL', 'IGNORE', 'REPLACE', 'DEFERRED', 'IMMEDIATE', 'EXCLUSIVE',
  'TEMP', 'WITHOUT', 'ROWID',
];

const SQLITE_FUNCTIONS = [
  'DATE', 'TIME', 'DATETIME', 'JULIANDAY', 'STRFTIME', 'UNIXEPOCH',
  'TYPEOF', 'PRINTF', 'FORMAT', 'QUOTE', 'HEX', 'RANDOM', 'RANDOMBLOB',
  'IFNULL', 'NULLIF', 'COALESCE', 'IIF',
  'GROUP_CONCAT', 'TOTAL', 'INSTR', 'CHANGES', 'TOTAL_CHANGES', 'LAST_INSERT_ROWID',
  'JSON', 'JSON_ARRAY', 'JSON_OBJECT', 'JSON_EXTRACT', 'JSON_TYPE', 'JSON_VALID',
  'JSON_ARRAY_LENGTH', 'JSON_INSERT', 'JSON_REPLACE', 'JSON_SET', 'JSON_REMOVE',
];

const SQLSERVER_KEYWORDS = [
  'TOP', 'NOLOCK', 'IDENTITY', 'NVARCHAR', 'NCHAR', 'NTEXT', 'UNIQUEIDENTIFIER', 'BIT',
  'MONEY', 'SMALLMONEY', 'IMAGE', 'DATETIMEOFFSET', 'DATETIME2', 'SMALLDATETIME', 'HIERARCHYID',
  'SQL_VARIANT', 'XML', 'GEOGRAPHY', 'GEOMETRY', 'ROWGUIDCOL', 'MERGE', 'OUTPUT',
  'APPLY', 'PIVOT', 'UNPIVOT', 'TRY', 'CATCH', 'THROW', 'RAISERROR', 'PRINT', 'EXEC',
  'EXECUTE', 'PROC', 'PROCEDURE', 'TRIGGER', 'CURSOR', 'FETCH', 'OPEN', 'CLOSE', 'DEALLOCATE',
  'GO', 'OFFSET', 'ROWS', 'FETCH', 'NEXT', 'ONLY',
];

const SQLSERVER_FUNCTIONS = [
  'GETDATE', 'GETUTCDATE', 'SYSDATETIME', 'SYSDATETIMEOFFSET', 'SYSUTCDATETIME',
  'DATEADD', 'DATEDIFF', 'DATEPART', 'DATENAME', 'DAY', 'MONTH', 'YEAR', 'EOMONTH',
  'CONVERT', 'CAST', 'TRY_CAST', 'TRY_CONVERT', 'TRY_PARSE', 'PARSE',
  'LEN', 'DATALENGTH', 'CHARINDEX', 'PATINDEX', 'SUBSTRING', 'STUFF', 'STRING_AGG', 'STRING_SPLIT',
  'CONCAT', 'CONCAT_WS', 'FORMAT', 'QUOTENAME', 'REVERSE', 'REPLICATE', 'SPACE',
  'UPPER', 'LOWER', 'LTRIM', 'RTRIM', 'TRIM', 'REPLACE',
  'ISNULL', 'NULLIF', 'COALESCE', 'IIF', 'CHOOSE',
  'NEWID', 'NEWSEQUENTIALID', 'HASHBYTES', 'CHECKSUM',
  'ROW_NUMBER', 'RANK', 'DENSE_RANK', 'NTILE', 'LAG', 'LEAD', 'FIRST_VALUE', 'LAST_VALUE',
  'OBJECT_ID', 'OBJECT_NAME', 'DB_NAME', 'DB_ID', 'USER_NAME', 'SUSER_NAME', 'SCHEMA_NAME',
  '@@ROWCOUNT', '@@IDENTITY', '@@VERSION', '@@SERVERNAME', '@@SPID',
];

const CLICKHOUSE_KEYWORDS = [
  'PREWHERE', 'FORMAT', 'ENGINE', 'MERGETREE', 'REPLACINGMERGETREE', 'AGGREGATINGMERGETREE',
  'SUMMINGMERGETREE', 'COLLAPSINGMERGETREE', 'PARTITION', 'SAMPLE', 'FINAL', 'TTL', 'CODEC',
  'LOWCARDINALITY', 'NULLABLE', 'ARRAY', 'TUPLE', 'MAP', 'NESTED', 'MATERIALIZED', 'ALIAS',
  'SETTINGS', 'CLUSTER', 'DICTIONARY', 'OPTIMIZE', 'SYSTEM', 'KILL', 'SHOW', 'DESCRIBE',
  'LIMIT', 'BY', 'TOTALS', 'ARRAY JOIN',
];

const CLICKHOUSE_FUNCTIONS = [
  'COUNT', 'COUNTIF', 'SUMIF', 'AVGIF', 'UNIQ', 'UNIQEXACT', 'GROUPARRAY', 'GROUPUNIQARRAY',
  'ARGMIN', 'ARGMAX', 'QUANTILE', 'QUANTILEEXACT', 'TOPK', 'ANY', 'ANYLAST',
  'TODATE', 'TODATETIME', 'TODATETIME64', 'TOSTARTOFHOUR', 'TOSTARTOFDAY', 'TOSTARTOFMONTH',
  'DATEADD', 'DATEDIFF', 'FORMATDATETIME', 'PARSEDATETIMEBESTEFFORT', 'NOW64',
  'ARRAYMAP', 'ARRAYFILTER', 'ARRAYJOIN', 'ARRAYEXISTS', 'ARRAYREDUCE', 'HAS', 'HASANY',
  'TYPENAME', 'TOSTRING', 'TOINT64', 'TOUINT64', 'TOFLOAT64', 'TODECIMAL64',
  'JSONEXTRACT', 'JSONEXTRACTSTRING', 'JSONEXTRACTINT', 'SIMPLEJSONEXTRACTSTRING',
  'CITYHASH64', 'SIPHASH64', 'GENERATEUUIDV4', 'MULTIIF', 'ASSUMENOTNULL',
];

export type SqlDialect =
  | 'mysql'
  | 'postgresql'
  | 'sqlite'
  | 'sqlserver'
  | 'mongodb'
  | 'redis'
  | 'clickhouse';

export function getDialectKeywords(dialect: SqlDialect): string[] {
  switch (dialect) {
    case 'mysql': return [...COMMON_SQL_KEYWORDS, ...MYSQL_KEYWORDS];
    case 'postgresql': return [...COMMON_SQL_KEYWORDS, ...POSTGRES_KEYWORDS];
    case 'sqlite': return [...COMMON_SQL_KEYWORDS, ...SQLITE_KEYWORDS];
    case 'sqlserver': return [...COMMON_SQL_KEYWORDS, ...SQLSERVER_KEYWORDS];
    case 'clickhouse': return [...COMMON_SQL_KEYWORDS, ...CLICKHOUSE_KEYWORDS];
    default: return [];
  }
}

export function getDialectFunctions(dialect: SqlDialect): string[] {
  switch (dialect) {
    case 'mysql': return [...COMMON_SQL_FUNCTIONS, ...MYSQL_FUNCTIONS];
    case 'postgresql': return [...COMMON_SQL_FUNCTIONS, ...POSTGRES_FUNCTIONS];
    case 'sqlite': return [...COMMON_SQL_FUNCTIONS, ...SQLITE_FUNCTIONS];
    case 'sqlserver': return [...COMMON_SQL_FUNCTIONS, ...SQLSERVER_FUNCTIONS];
    case 'clickhouse': return [...COMMON_SQL_FUNCTIONS, ...CLICKHOUSE_FUNCTIONS];
    default: return [];
  }
}

const registeredDialects = new Set<string>();

// --- Identifier quoting helper ---
// Returns the identifier wrapped with the right quoting rules for the dialect when needed.
// If the identifier is a simple ASCII identifier, returns it as-is.
function needsQuoting(ident: string): boolean {
  // Simple ident: starts with letter/underscore, only contains letters/digits/underscore
  return !/^[A-Za-z_][A-Za-z0-9_]*$/.test(ident);
}

function quoteIdentifier(ident: string, dbType: string): string {
  if (!needsQuoting(ident)) return ident;
  switch (dbType) {
    case 'mysql':
      return `\`${ident.replace(/`/g, '``')}\``;
    case 'clickhouse':
      return quoteClickHouseIdentifier(ident);
    case 'sqlserver':
      return `[${ident.replace(/]/g, ']]')}]`;
    case 'postgresql':
    case 'sqlite':
      return `"${ident.replace(/"/g, '""')}"`;
    default:
      return ident;
  }
}

function formatTableInsert(schema: string | undefined, name: string, dbType: string): string {
  const quotedName = quoteIdentifier(name, dbType);
  if (!schema) return quotedName;
  return `${quoteIdentifier(schema, dbType)}.${quotedName}`;
}

// --- Database-aware autocompletion ---

export interface TableCompletionData {
  tables: Array<{ name: string; schema?: string; columns: Array<{ name: string; type: string }> }>;
}

let dbCompletionDisposable: any = null;

export function registerDatabaseCompletions(
  monaco: typeof import('monaco-editor'),
  data: TableCompletionData,
  dbType: string,
) {
  // Dispose previous registration to avoid duplicates
  if (dbCompletionDisposable) {
    dbCompletionDisposable.dispose();
    dbCompletionDisposable = null;
  }

  dbCompletionDisposable = monaco.languages.registerCompletionItemProvider('sql', {
    triggerCharacters: ['.', ' '],
    provideCompletionItems: (model, position) => {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      // Check if the user typed a table name followed by "."
      const lineContent = model.getLineContent(position.lineNumber);
      const textBeforeCursor = lineContent.substring(0, position.column - 1);
      const dotMatch = textBeforeCursor.match(/(\w+)\.$/);

      if (dotMatch) {
        // After a dot — suggest columns for that table
        const tableName = dotMatch[1].toLowerCase();
        const table = data.tables.find(t =>
          t.name.toLowerCase() === tableName ||
          (t.schema && `${t.schema}.${t.name}`.toLowerCase() === tableName)
        );
        if (table) {
          return {
            suggestions: table.columns.map(col => ({
              label: col.name,
              kind: monaco.languages.CompletionItemKind.Field,
              insertText: needsQuoting(col.name) ? quoteIdentifier(col.name, dbType) : col.name,
              detail: col.type,
              range,
            })),
          };
        }

        // Could also be a schema name — suggest tables in that schema
        const schemaName = dotMatch[1].toLowerCase();
        const schemaTables = data.tables.filter(t => t.schema?.toLowerCase() === schemaName);
        if (schemaTables.length > 0) {
          return {
            suggestions: schemaTables.map(t => ({
              label: t.name,
              kind: monaco.languages.CompletionItemKind.Class,
              insertText: needsQuoting(t.name) ? quoteIdentifier(t.name, dbType) : t.name,
              detail: `${t.schema}.${t.name}`,
              range,
            })),
          };
        }
      }

      // Default — suggest table names (and schema names for PG)
      const suggestions: any[] = [];

      // Add table names — only quote when the identifier truly needs quoting
      data.tables.forEach(t => {
        suggestions.push({
          label: t.schema ? `${t.schema}.${t.name}` : t.name,
          kind: monaco.languages.CompletionItemKind.Class,
          insertText: formatTableInsert(t.schema, t.name, dbType),
          detail: 'table',
          range,
        });
        // Also add just the table name for convenience
        if (t.schema) {
          suggestions.push({
            label: t.name,
            kind: monaco.languages.CompletionItemKind.Class,
            insertText: needsQuoting(t.name) ? quoteIdentifier(t.name, dbType) : t.name,
            detail: `${t.schema}.${t.name}`,
            range,
          });
        }
      });

      // Add schema names for PG
      const schemas = [...new Set(data.tables.map(t => t.schema).filter(Boolean))];
      schemas.forEach(s => {
        suggestions.push({
          label: s!,
          kind: monaco.languages.CompletionItemKind.Module,
          insertText: needsQuoting(s!) ? quoteIdentifier(s!, dbType) : s!,
          detail: 'schema',
          range,
        });
      });

      // Add all column names as general suggestions
      const seenColumns = new Set<string>();
      data.tables.forEach(t => {
        t.columns.forEach(col => {
          if (!seenColumns.has(col.name)) {
            seenColumns.add(col.name);
            suggestions.push({
              label: col.name,
              kind: monaco.languages.CompletionItemKind.Field,
              insertText: needsQuoting(col.name) ? quoteIdentifier(col.name, dbType) : col.name,
              detail: `column (${col.type})`,
              range,
              sortText: '1_' + col.name, // Sort after tables
            });
          }
        });
      });

      return { suggestions };
    },
  });
}

export function clearDatabaseCompletions() {
  if (dbCompletionDisposable) {
    dbCompletionDisposable.dispose();
    dbCompletionDisposable = null;
  }
}

// --- Dialect keyword & function completion ---

export function configureMonacoForDialect(
  monaco: typeof import('monaco-editor'),
  dialect: SqlDialect
) {
  const key = `sql-${dialect}`;
  if (registeredDialects.has(key)) return;
  registeredDialects.add(key);

  const keywords = getDialectKeywords(dialect);
  const functions = getDialectFunctions(dialect);
  if (keywords.length === 0 && functions.length === 0) return;

  monaco.languages.registerCompletionItemProvider('sql', {
    provideCompletionItems: (model, position) => {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      const keywordSuggestions: languages.CompletionItem[] = keywords.map((kw) => ({
        label: kw,
        kind: monaco.languages.CompletionItemKind.Keyword,
        insertText: kw,
        range,
        detail: `${dialect.toUpperCase()} keyword`,
      }));

      const functionSuggestions: languages.CompletionItem[] = functions.map((fn) => ({
        label: fn,
        kind: monaco.languages.CompletionItemKind.Function,
        insertText: `${fn}($0)`,
        insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
        range,
        detail: `${dialect.toUpperCase()} function`,
      }));

      return { suggestions: [...keywordSuggestions, ...functionSuggestions] };
    },
  });
}
