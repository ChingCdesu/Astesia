import { useState, useEffect, useMemo, useCallback } from 'react';
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  ReactFlowProvider,
  type Node,
  type Edge,
  useNodesState,
  useEdgesState,
  BackgroundVariant,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import dagre from '@dagrejs/dagre';
import { invoke } from '@tauri-apps/api/core';
import { Loader2, GitFork } from 'lucide-react';
import { useThemeStore } from '@/stores/themeStore';
import type { TableInfo, ColumnInfo } from '@/types/database';
import TableNode, {
  HEADER_HEIGHT,
  ROW_HEIGHT,
  type TableNodeData,
  type TableColumn,
} from './TableNode';

interface ForeignKeyInfo {
  name: string;
  from_table: string;
  from_columns: string[];
  to_table: string;
  to_columns: string[];
}

interface Props {
  connectionId: string;
  database: string;
}

const NODE_WIDTH = 250;

function getNodeHeight(columnCount: number): number {
  return HEADER_HEIGHT + columnCount * ROW_HEIGHT;
}

function applyDagreLayout(
  nodes: Node<TableNodeData>[],
  edges: Edge[]
): Node<TableNodeData>[] {
  const g = new dagre.graphlib.Graph();
  g.setDefaultEdgeLabel(() => ({}));
  g.setGraph({ rankdir: 'LR', nodesep: 80, ranksep: 150, marginx: 40, marginy: 40 });

  nodes.forEach((node) => {
    const height = getNodeHeight(node.data.columns.length);
    g.setNode(node.id, { width: NODE_WIDTH, height });
  });

  edges.forEach((edge) => {
    g.setEdge(edge.source, edge.target);
  });

  dagre.layout(g);

  return nodes.map((node) => {
    const pos = g.node(node.id);
    const height = getNodeHeight(node.data.columns.length);
    return {
      ...node,
      position: {
        x: pos.x - NODE_WIDTH / 2,
        y: pos.y - height / 2,
      },
    };
  });
}

function ERDiagramInner({ connectionId, database }: Props) {
  const resolvedTheme = useThemeStore((s) => s.resolvedTheme);
  const [nodes, setNodes, onNodesChange] = useNodesState<Node<TableNodeData>>([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const nodeTypes = useMemo(() => ({ tableNode: TableNode }), []);

  const defaultEdgeOptions = useMemo(
    () => ({
      type: 'smoothstep' as const,
      animated: true,
      style: { strokeWidth: 1.5 },
    }),
    []
  );

  const loadDiagram = useCallback(async () => {
    setLoading(true);
    setError(null);

    try {
      // Step 1: Load all tables
      const tables = await invoke<TableInfo[]>('get_tables', {
        connectionId,
        database,
      });

      if (!tables || tables.length === 0) {
        setNodes([]);
        setEdges([]);
        setLoading(false);
        return;
      }

      // Step 2: Load columns and foreign keys for every table in parallel
      const [allColumns, allForeignKeys] = await Promise.all([
        Promise.all(
          tables.map((t) =>
            invoke<ColumnInfo[]>('get_columns', {
              connectionId,
              database,
              table: t.name,
            }).then((cols) => ({ table: t.name, columns: cols }))
          )
        ),
        Promise.all(
          tables.map((t) =>
            invoke<ForeignKeyInfo[]>('get_foreign_keys', {
              connectionId,
              database,
              table: t.name,
            })
              .then((fks) => ({ table: t.name, foreignKeys: fks }))
              .catch(() => ({ table: t.name, foreignKeys: [] as ForeignKeyInfo[] }))
          )
        ),
      ]);

      // Build a map of table -> column info
      const columnsMap = new Map<string, ColumnInfo[]>();
      allColumns.forEach(({ table, columns }) => {
        columnsMap.set(table, columns);
      });

      // Collect all FK info and build a set of FK columns per table
      const allFKs: ForeignKeyInfo[] = [];
      const fkColumnMap = new Map<string, Set<string>>();

      allForeignKeys.forEach(({ foreignKeys }) => {
        foreignKeys.forEach((fk) => {
          allFKs.push(fk);
          const existing = fkColumnMap.get(fk.from_table) || new Set<string>();
          fk.from_columns.forEach((col) => existing.add(col));
          fkColumnMap.set(fk.from_table, existing);
        });
      });

      // Step 3: Create nodes
      const initialNodes: Node<TableNodeData>[] = tables.map((t) => {
        const cols = columnsMap.get(t.name) || [];
        const fkCols = fkColumnMap.get(t.name) || new Set<string>();

        const columns: TableColumn[] = cols.map((col) => ({
          name: col.name,
          type: col.data_type,
          isPrimaryKey: col.is_primary_key,
          isForeignKey: fkCols.has(col.name),
        }));

        return {
          id: t.name,
          type: 'tableNode',
          position: { x: 0, y: 0 },
          data: {
            label: t.name,
            columns,
          },
        };
      });

      // Step 4: Create edges (deduplicate by FK name)
      const seenFKs = new Set<string>();
      const initialEdges: Edge[] = [];

      allFKs.forEach((fk) => {
        const edgeId = fk.name || `${fk.from_table}_${fk.from_columns.join('_')}_${fk.to_table}`;
        if (seenFKs.has(edgeId)) return;
        seenFKs.add(edgeId);

        // Find the source and target handle IDs based on column names
        const sourceHandle = fk.from_columns[0]
          ? `${fk.from_columns[0]}-source`
          : undefined;
        const targetHandle = fk.to_columns[0]
          ? `${fk.to_columns[0]}-target`
          : undefined;

        initialEdges.push({
          id: edgeId,
          source: fk.from_table,
          target: fk.to_table,
          sourceHandle,
          targetHandle,
          label: fk.from_columns.join(', '),
          labelStyle: { fontSize: 10, fill: 'var(--color-muted-foreground)' },
          labelBgStyle: {
            fill: 'var(--color-card)',
            stroke: 'var(--color-border)',
            strokeWidth: 0.5,
            rx: 3,
            ry: 3,
          },
          labelBgPadding: [4, 2] as [number, number],
          labelShowBg: true,
        });
      });

      // Step 5: Apply dagre layout
      const layoutedNodes = applyDagreLayout(initialNodes, initialEdges);

      setNodes(layoutedNodes);
      setEdges(initialEdges);
    } catch (e) {
      console.error('Failed to load ER diagram:', e);
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [connectionId, database, setNodes, setEdges]);

  useEffect(() => {
    loadDiagram();
  }, [loadDiagram]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
        <span>正在加载 ER 图...</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
        <span className="text-sm text-destructive">加载失败: {error}</span>
        <button
          onClick={loadDiagram}
          className="rounded-md bg-secondary px-3 py-1.5 text-xs text-secondary-foreground hover:bg-secondary/80"
        >
          重试
        </button>
      </div>
    );
  }

  if (nodes.length === 0) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
        <GitFork className="h-10 w-10 opacity-20" />
        <span className="text-sm">未发现表或外键关系</span>
      </div>
    );
  }

  return (
    <div className="h-full w-full">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        nodeTypes={nodeTypes}
        defaultEdgeOptions={defaultEdgeOptions}
        colorMode={resolvedTheme}
        fitView
        fitViewOptions={{ padding: 0.15 }}
        minZoom={0.1}
        maxZoom={2}
        proOptions={{ hideAttribution: true }}
      >
        <Background variant={BackgroundVariant.Dots} gap={16} size={1} />
        <Controls
          showInteractive={false}
          className="!rounded-lg !border !border-border !bg-card !shadow-sm"
        />
        <MiniMap
          className="!rounded-lg !border !border-border !bg-card !shadow-sm"
          nodeColor="var(--color-muted)"
          maskColor="var(--color-background)"
          style={{ width: 150, height: 100 }}
        />
      </ReactFlow>
    </div>
  );
}

export default function ERDiagram(props: Props) {
  return (
    <ReactFlowProvider>
      <ERDiagramInner {...props} />
    </ReactFlowProvider>
  );
}
