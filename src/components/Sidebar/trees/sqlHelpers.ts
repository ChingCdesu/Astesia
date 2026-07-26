/**
 * DB-type-aware SQL generation helpers for table/schema operations.
 */
import { quoteClickHouseIdentifier } from '@/lib/sqlIdentifier';

export function getDropTableSQL(table: string, dbType: string): string {
  switch (dbType) {
    case 'postgresql': {
      const parts = table.split('.');
      if (parts.length === 2) return `DROP TABLE "${parts[0]}"."${parts[1]}"`;
      return `DROP TABLE "${table}"`;
    }
    case 'mysql':
      return `DROP TABLE \`${table}\``;
    case 'clickhouse':
      return `DROP TABLE ${quoteClickHouseIdentifier(table)}`;
    case 'sqlserver':
      return `DROP TABLE [${table}]`;
    default:
      return `DROP TABLE "${table}"`;
  }
}

export function getRenameTableSQL(oldName: string, newName: string, dbType: string): string {
  switch (dbType) {
    case 'postgresql': {
      const parts = oldName.split('.');
      if (parts.length === 2) return `ALTER TABLE "${parts[0]}"."${parts[1]}" RENAME TO "${newName}"`;
      return `ALTER TABLE "${oldName}" RENAME TO "${newName}"`;
    }
    case 'mysql':
      return `RENAME TABLE \`${oldName}\` TO \`${newName}\``;
    case 'clickhouse':
      return `RENAME TABLE ${quoteClickHouseIdentifier(oldName)} TO ${quoteClickHouseIdentifier(newName)}`;
    case 'sqlserver':
      return `EXEC sp_rename '${oldName}', '${newName}'`;
    default:
      return `ALTER TABLE "${oldName}" RENAME TO "${newName}"`;
  }
}

export function getDropViewSQL(viewName: string, dbType: string): string {
  switch (dbType) {
    case 'postgresql': {
      const parts = viewName.split('.');
      if (parts.length === 2) return `DROP VIEW "${parts[0]}"."${parts[1]}"`;
      return `DROP VIEW "${viewName}"`;
    }
    case 'mysql':
      return `DROP VIEW \`${viewName}\``;
    case 'clickhouse':
      return `DROP VIEW ${quoteClickHouseIdentifier(viewName)}`;
    case 'sqlserver':
      return `DROP VIEW [${viewName}]`;
    default:
      return `DROP VIEW "${viewName}"`;
  }
}

export function getDropFunctionSQL(funcName: string, dbType: string): string {
  switch (dbType) {
    case 'postgresql': {
      const parts = funcName.split('.');
      if (parts.length === 2) return `DROP FUNCTION "${parts[0]}"."${parts[1]}"`;
      return `DROP FUNCTION "${funcName}"`;
    }
    case 'mysql':
      return `DROP FUNCTION \`${funcName}\``;
    case 'clickhouse':
      return `DROP FUNCTION ${quoteClickHouseIdentifier(funcName)}`;
    case 'sqlserver':
      return `DROP FUNCTION [${funcName}]`;
    default:
      return `DROP FUNCTION "${funcName}"`;
  }
}

export function getDropProcedureSQL(procName: string, dbType: string): string {
  switch (dbType) {
    case 'postgresql': {
      const parts = procName.split('.');
      if (parts.length === 2) return `DROP PROCEDURE "${parts[0]}"."${parts[1]}"`;
      return `DROP PROCEDURE "${procName}"`;
    }
    case 'mysql':
      return `DROP PROCEDURE \`${procName}\``;
    case 'sqlserver':
      return `DROP PROCEDURE [${procName}]`;
    default:
      return `DROP PROCEDURE "${procName}"`;
  }
}

export function getDropTriggerSQL(triggerName: string, dbType: string): string {
  switch (dbType) {
    case 'postgresql': {
      const parts = triggerName.split('.');
      if (parts.length === 2) return `DROP TRIGGER "${parts[0]}"."${parts[1]}" ON ""`;
      return `DROP TRIGGER "${triggerName}"`;
    }
    case 'mysql':
      return `DROP TRIGGER \`${triggerName}\``;
    case 'sqlserver':
      return `DROP TRIGGER [${triggerName}]`;
    default:
      return `DROP TRIGGER "${triggerName}"`;
  }
}

export function getDropSchemaSQL(schemaName: string): string {
  return `DROP SCHEMA "${schemaName}" CASCADE`;
}

export function getRenameSchemaSQL(oldName: string, newName: string): string {
  return `ALTER SCHEMA "${oldName}" RENAME TO "${newName}"`;
}
