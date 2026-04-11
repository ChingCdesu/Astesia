import { memo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';
import { cn } from '@/lib/utils';

export interface TableColumn {
  name: string;
  type: string;
  isPrimaryKey: boolean;
  isForeignKey: boolean;
}

export interface TableNodeData {
  label: string;
  columns: TableColumn[];
  [key: string]: unknown;
}

const HEADER_HEIGHT = 32;
const ROW_HEIGHT = 28;

const TableNode = memo(({ data }: NodeProps) => {
  const { label, columns } = data as TableNodeData;

  return (
    <div className="min-w-[220px] overflow-hidden rounded-lg border border-border bg-card shadow-md">
      {/* Table header */}
      <div
        className="flex items-center justify-center bg-primary px-3 font-semibold text-primary-foreground"
        style={{ height: HEADER_HEIGHT }}
      >
        <span className="truncate text-xs">{label}</span>
      </div>

      {/* Columns */}
      <div className="relative">
        {columns.map((col: TableColumn, index: number) => {
          const topOffset = HEADER_HEIGHT + index * ROW_HEIGHT + ROW_HEIGHT / 2;

          return (
            <div
              key={col.name}
              className={cn(
                'flex items-center gap-1.5 border-b border-border/50 px-3 text-xs last:border-b-0',
                col.isPrimaryKey && 'bg-amber-500/5',
                col.isForeignKey && 'bg-blue-500/5'
              )}
              style={{ height: ROW_HEIGHT }}
            >
              {/* Badges */}
              <div className="flex w-8 shrink-0 gap-0.5">
                {col.isPrimaryKey && (
                  <span className="rounded px-1 py-px text-[9px] font-bold leading-tight text-amber-600 dark:text-amber-400">
                    PK
                  </span>
                )}
                {col.isForeignKey && (
                  <span className="rounded px-1 py-px text-[9px] font-bold leading-tight text-blue-600 dark:text-blue-400">
                    FK
                  </span>
                )}
              </div>

              {/* Column name */}
              <span
                className={cn(
                  'flex-1 truncate font-mono text-card-foreground',
                  col.isPrimaryKey && 'font-semibold'
                )}
              >
                {col.name}
              </span>

              {/* Column type */}
              <span className="shrink-0 text-[10px] text-muted-foreground">
                {col.type}
              </span>

              {/* Handles per column row */}
              <Handle
                type="target"
                position={Position.Left}
                id={`${col.name}-target`}
                className="!h-2 !w-2 !min-h-0 !min-w-0 !rounded-full !border !border-border !bg-muted-foreground/40"
                style={{ top: topOffset, left: -4 }}
                isConnectable={false}
              />
              <Handle
                type="source"
                position={Position.Right}
                id={`${col.name}-source`}
                className="!h-2 !w-2 !min-h-0 !min-w-0 !rounded-full !border !border-border !bg-muted-foreground/40"
                style={{ top: topOffset, right: -4 }}
                isConnectable={false}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
});

TableNode.displayName = 'TableNode';

export default TableNode;
export { HEADER_HEIGHT, ROW_HEIGHT };
