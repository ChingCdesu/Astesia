import type { ReactNode } from 'react';
import type { QueryResult, ColumnInfo, DbType } from '@/types/database';

/**
 * Plugin registration for custom cell value viewers.
 * A CellValueViewer plugin can render a custom inline preview
 * and/or a full-screen viewer for specific data types.
 */
export interface CellValueViewerPlugin {
  /** Unique plugin ID */
  id: string;
  /** Display name */
  name: string;
  /** Check if this plugin can handle the given column type */
  canHandle: (columnType: string, dbType: DbType) => boolean;
  /** Priority (higher wins when multiple plugins match). Built-in = 0 */
  priority: number;
  /** Render an inline cell preview (optional) */
  renderInline?: (value: any, columnType: string) => ReactNode;
  /** Render the full viewer dialog content (optional) */
  renderViewer?: (props: {
    value: any;
    columnType: string;
    columnName: string;
    onChange?: (newValue: any) => void;
  }) => ReactNode;
}

/**
 * Plugin registration for custom DataViewer tab types.
 * A DataViewer plugin provides an entire tab view for a database connection.
 */
export interface DataViewerPlugin {
  /** Unique plugin ID */
  id: string;
  /** Display name */
  name: string;
  /** Icon component */
  icon?: ReactNode;
  /** Which database types this viewer supports */
  supportedDbTypes: DbType[] | 'all';
  /** Render the viewer component */
  render: (props: {
    connectionId: string;
    database: string;
    table?: string;
  }) => ReactNode;
}

/**
 * Plugin registration for sidebar tree extensions.
 * Adds custom nodes or actions to the sidebar tree.
 */
export interface SidebarPlugin {
  /** Unique plugin ID */
  id: string;
  /** Display name */
  name: string;
  /** Which database types this applies to */
  supportedDbTypes: DbType[] | 'all';
  /** Additional context menu items for database nodes */
  databaseMenuItems?: (props: {
    connectionId: string;
    database: string;
    dbType: DbType;
  }) => ReactNode;
  /** Additional context menu items for table nodes */
  tableMenuItems?: (props: {
    connectionId: string;
    database: string;
    table: string;
    dbType: DbType;
  }) => ReactNode;
}
