import type { languages } from 'monaco-editor';

// Dialect-specific keywords
const MYSQL_KEYWORDS = ['SHOW', 'DATABASES', 'TABLES', 'DESCRIBE', 'EXPLAIN', 'USE', 'ENGINE', 'AUTO_INCREMENT', 'CHARSET', 'COLLATE', 'IFNULL', 'LIMIT', 'OFFSET', 'REGEXP', 'BINARY', 'UNSIGNED', 'ZEROFILL', 'ENUM', 'SET', 'TINYINT', 'SMALLINT', 'MEDIUMINT', 'BIGINT', 'FLOAT', 'DOUBLE', 'DECIMAL', 'DATETIME', 'TIMESTAMP', 'YEAR', 'TINYTEXT', 'MEDIUMTEXT', 'LONGTEXT', 'TINYBLOB', 'MEDIUMBLOB', 'LONGBLOB', 'JSON'];

const POSTGRES_KEYWORDS = ['RETURNING', 'ILIKE', 'SIMILAR', 'ARRAY', 'JSONB', 'HSTORE', 'SERIAL', 'BIGSERIAL', 'SMALLSERIAL', 'BYTEA', 'UUID', 'INET', 'CIDR', 'MACADDR', 'MONEY', 'INTERVAL', 'TSQUERY', 'TSVECTOR', 'REGCLASS', 'LATERAL', 'MATERIALIZED', 'REFRESH', 'EXTENSION', 'SCHEMA', 'CONCURRENTLY', 'VACUUM', 'ANALYZE', 'REINDEX', 'CLUSTER', 'NOTIFY', 'LISTEN', 'UNLISTEN', 'COPY', 'EXCLUDE', 'PARTITION', 'INHERIT', 'RULE'];

const SQLITE_KEYWORDS = ['PRAGMA', 'AUTOINCREMENT', 'GLOB', 'VACUUM', 'ATTACH', 'DETACH', 'REINDEX', 'INDEXED', 'CONFLICT', 'ABORT', 'FAIL', 'IGNORE', 'REPLACE', 'ROLLBACK', 'DEFERRED', 'IMMEDIATE', 'EXCLUSIVE', 'TEMP', 'WITHOUT', 'ROWID'];

const SQLSERVER_KEYWORDS = ['TOP', 'NOLOCK', 'IDENTITY', 'NVARCHAR', 'NCHAR', 'NTEXT', 'UNIQUEIDENTIFIER', 'BIT', 'MONEY', 'SMALLMONEY', 'IMAGE', 'DATETIMEOFFSET', 'DATETIME2', 'SMALLDATETIME', 'HIERARCHYID', 'SQL_VARIANT', 'XML', 'GEOGRAPHY', 'GEOMETRY', 'ROWGUIDCOL', 'MERGE', 'OUTPUT', 'CROSS', 'APPLY', 'OUTER', 'PIVOT', 'UNPIVOT', 'TRY', 'CATCH', 'THROW', 'RAISERROR', 'PRINT', 'EXEC', 'EXECUTE', 'PROC', 'PROCEDURE', 'TRIGGER', 'CURSOR', 'FETCH', 'OPEN', 'CLOSE', 'DEALLOCATE'];

export type SqlDialect = 'mysql' | 'postgresql' | 'sqlite' | 'sqlserver' | 'mongodb' | 'redis';

export function getDialectKeywords(dialect: SqlDialect): string[] {
  switch (dialect) {
    case 'mysql': return MYSQL_KEYWORDS;
    case 'postgresql': return POSTGRES_KEYWORDS;
    case 'sqlite': return SQLITE_KEYWORDS;
    case 'sqlserver': return SQLSERVER_KEYWORDS;
    default: return [];
  }
}

const registeredDialects = new Set<string>();

export function configureMonacoForDialect(
  monaco: typeof import('monaco-editor'),
  dialect: SqlDialect
) {
  const key = `sql-${dialect}`;
  if (registeredDialects.has(key)) return;
  registeredDialects.add(key);

  const keywords = getDialectKeywords(dialect);
  if (keywords.length === 0) return;

  monaco.languages.registerCompletionItemProvider('sql', {
    provideCompletionItems: (model, position) => {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      const suggestions: languages.CompletionItem[] = keywords.map((kw) => ({
        label: kw,
        kind: monaco.languages.CompletionItemKind.Keyword,
        insertText: kw,
        range,
        detail: `${dialect.toUpperCase()} keyword`,
      }));

      return { suggestions };
    },
  });
}
