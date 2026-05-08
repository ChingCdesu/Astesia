// Split a SQL string into individual statements, ignoring semicolons inside
// quoted strings, identifier quotes, and comments.
//
// Supported quoting rules:
// - Single quotes ('...') with backslash and SQL '' escapes
// - Double quotes ("...") used as identifier quotes for PostgreSQL/SQLite
// - Backticks (`...`) used as identifier quotes for MySQL
// - Square brackets ([...]) used as identifier quotes for SQL Server
// - $$ ... $$ dollar-quoted strings (PostgreSQL function bodies)
// - -- line comments
// - /* ... */ block comments

export interface SqlStatement {
  sql: string;
  startLine: number;
}

export function splitSqlStatements(input: string): SqlStatement[] {
  const out: SqlStatement[] = [];
  let buf = '';
  let bufStartLine = 1;
  let line = 1;
  let i = 0;
  const len = input.length;

  // Track current line where buffer started; advance line counter as we walk
  const flush = () => {
    const trimmed = buf.trim();
    if (trimmed) {
      out.push({ sql: trimmed, startLine: bufStartLine });
    }
    buf = '';
    bufStartLine = line;
  };

  while (i < len) {
    const ch = input[i];
    const next = i + 1 < len ? input[i + 1] : '';

    // Line comment -- ... \n
    if (ch === '-' && next === '-') {
      while (i < len && input[i] !== '\n') {
        buf += input[i];
        i++;
      }
      continue;
    }

    // Block comment /* ... */
    if (ch === '/' && next === '*') {
      buf += ch;
      buf += next;
      i += 2;
      while (i < len && !(input[i] === '*' && input[i + 1] === '/')) {
        if (input[i] === '\n') line++;
        buf += input[i];
        i++;
      }
      if (i < len) {
        buf += input[i];
        buf += input[i + 1];
        i += 2;
      }
      continue;
    }

    // Dollar-quoted string $tag$...$tag$
    if (ch === '$') {
      const tagMatch = input.slice(i).match(/^\$([A-Za-z_][A-Za-z0-9_]*)?\$/);
      if (tagMatch) {
        const tag = tagMatch[0];
        buf += tag;
        i += tag.length;
        while (i < len && input.slice(i, i + tag.length) !== tag) {
          if (input[i] === '\n') line++;
          buf += input[i];
          i++;
        }
        if (i < len) {
          buf += tag;
          i += tag.length;
        }
        continue;
      }
    }

    // Quoted strings
    if (ch === "'" || ch === '"' || ch === '`') {
      const quote = ch;
      buf += ch;
      i++;
      while (i < len) {
        const c = input[i];
        if (c === '\\' && i + 1 < len) {
          buf += c;
          buf += input[i + 1];
          if (input[i + 1] === '\n') line++;
          i += 2;
          continue;
        }
        if (c === quote) {
          // Doubled quote == escaped quote (SQL standard)
          if (i + 1 < len && input[i + 1] === quote) {
            buf += c;
            buf += input[i + 1];
            i += 2;
            continue;
          }
          buf += c;
          i++;
          break;
        }
        if (c === '\n') line++;
        buf += c;
        i++;
      }
      continue;
    }

    // SQL Server bracket-quoted identifier [...]
    if (ch === '[') {
      buf += ch;
      i++;
      while (i < len && input[i] !== ']') {
        if (input[i] === '\n') line++;
        buf += input[i];
        i++;
      }
      if (i < len) {
        buf += input[i];
        i++;
      }
      continue;
    }

    if (ch === ';') {
      flush();
      i++;
      // Swallow whitespace following the semicolon so the next statement
      // does not start with leading newlines that throw off bufStartLine
      while (i < len && /\s/.test(input[i])) {
        if (input[i] === '\n') line++;
        i++;
      }
      bufStartLine = line;
      continue;
    }

    if (ch === '\n') line++;
    buf += ch;
    i++;
  }

  flush();
  return out;
}

const TX_BEGIN = /^\s*(BEGIN|START\s+TRANSACTION)\b/i;
const TX_COMMIT = /^\s*COMMIT\b/i;
const TX_ROLLBACK = /^\s*ROLLBACK\b/i;
const TX_SAVEPOINT = /^\s*(SAVEPOINT|RELEASE\s+SAVEPOINT)\b/i;

export function isTransactionStart(sql: string): boolean {
  return TX_BEGIN.test(sql);
}

export function isTransactionEnd(sql: string): boolean {
  return TX_COMMIT.test(sql) || TX_ROLLBACK.test(sql);
}

export function isTransactionControl(sql: string): boolean {
  return isTransactionStart(sql) || isTransactionEnd(sql) || TX_SAVEPOINT.test(sql);
}

export function containsTransactionControl(statements: SqlStatement[]): boolean {
  return statements.some((s) => isTransactionControl(s.sql));
}
