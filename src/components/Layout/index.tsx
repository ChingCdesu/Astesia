import { useState, useCallback, useRef, useEffect } from 'react';
import { useTabStore } from '@/stores/tabStore';
import { useConnectionStore } from '@/stores/connectionStore';
import { confirm } from '@/stores/confirmStore';
import QueryEditor from '../QueryEditor';
import DataGrid from '../DataViewers/DataGrid';
import RedisViewer from '../DataViewers/RedisViewer';
import MongoViewer from '../DataViewers/MongoViewer';
import TableStructure from '../TableStructure';
import ObjectDefinition from '../ObjectDefinition';
import PerformanceDashboard from '../PerformanceDashboard';
import DataChartView from '../DataChartView';
import ERDiagram from '../ERDiagram';
import Sidebar from '../Sidebar';
import StatusBar from '../StatusBar';
import { Database, X } from 'lucide-react';
import { cn } from '@/lib/utils';
import {
  ContextMenu,
  ContextMenuTrigger,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
} from '@/components/ui/context-menu';

const MIN_SIDEBAR_WIDTH = 200;
const MAX_SIDEBAR_WIDTH = 600;
const DEFAULT_SIDEBAR_WIDTH = 272;

export default function AppLayout() {
  const { tabs, activeTabKey, setActiveTab, removeTab, removeAllTabs, removeOtherTabs, removeLeftTabs, removeRightTabs } = useTabStore();
  const connections = useConnectionStore((s) => s.connections);

  // Sidebar resize state
  const [sidebarWidth, setSidebarWidth] = useState(() => {
    const saved = localStorage.getItem('astesia_sidebar_width');
    return saved ? Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, parseInt(saved))) : DEFAULT_SIDEBAR_WIDTH;
  });
  const [isResizing, setIsResizing] = useState(false);
  const resizeRef = useRef({ startX: 0, startWidth: 0 });

  const handleResizeStart = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setIsResizing(true);
    resizeRef.current = { startX: e.clientX, startWidth: sidebarWidth };
  }, [sidebarWidth]);

  useEffect(() => {
    if (!isResizing) return;

    const handleMouseMove = (e: MouseEvent) => {
      const delta = e.clientX - resizeRef.current.startX;
      const newWidth = Math.max(MIN_SIDEBAR_WIDTH, Math.min(MAX_SIDEBAR_WIDTH, resizeRef.current.startWidth + delta));
      setSidebarWidth(newWidth);
    };

    const handleMouseUp = () => {
      setIsResizing(false);
      localStorage.setItem('astesia_sidebar_width', String(sidebarWidth));
    };

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
  }, [isResizing, sidebarWidth]);

  const hasUnsavedContent = useCallback((tab: (typeof tabs)[0]) => {
    return tab.type === 'query' && tab.sqlContent && tab.sqlContent.trim();
  }, []);

  const handleCloseTab = useCallback(async (key: string) => {
    const tab = tabs.find(t => t.key === key);
    if (tab && hasUnsavedContent(tab)) {
      const ok = await confirm('关闭标签页', '当前查询内容未保存，是否继续关闭？', 'default');
      if (!ok) return;
    }
    removeTab(key);
  }, [tabs, hasUnsavedContent, removeTab]);

  const handleCloseAllTabs = useCallback(async () => {
    const unsavedTabs = tabs.filter(t => hasUnsavedContent(t));
    if (unsavedTabs.length > 0) {
      const ok = await confirm('关闭标签页', '存在未保存的查询内容，是否继续关闭所有标签页？', 'default');
      if (!ok) return;
    }
    removeAllTabs();
  }, [tabs, hasUnsavedContent, removeAllTabs]);

  const handleCloseOtherTabs = useCallback(async (key: string) => {
    const otherUnsaved = tabs.filter(t => t.key !== key && hasUnsavedContent(t));
    if (otherUnsaved.length > 0) {
      const ok = await confirm('关闭标签页', '存在未保存的查询内容，是否继续关闭其他标签页？', 'default');
      if (!ok) return;
    }
    removeOtherTabs(key);
  }, [tabs, hasUnsavedContent, removeOtherTabs]);

  const handleCloseLeftTabs = useCallback(async (key: string) => {
    const idx = tabs.findIndex(t => t.key === key);
    const leftUnsaved = tabs.slice(0, idx).filter(t => hasUnsavedContent(t));
    if (leftUnsaved.length > 0) {
      const ok = await confirm('关闭标签页', '存在未保存的查询内容，是否继续关闭左边的标签页？', 'default');
      if (!ok) return;
    }
    removeLeftTabs(key);
  }, [tabs, hasUnsavedContent, removeLeftTabs]);

  const handleCloseRightTabs = useCallback(async (key: string) => {
    const idx = tabs.findIndex(t => t.key === key);
    const rightUnsaved = tabs.slice(idx + 1).filter(t => hasUnsavedContent(t));
    if (rightUnsaved.length > 0) {
      const ok = await confirm('关闭标签页', '存在未保存的查询内容，是否继续关闭右边的标签页？', 'default');
      if (!ok) return;
    }
    removeRightTabs(key);
  }, [tabs, hasUnsavedContent, removeRightTabs]);

  const renderTabContent = (tab: (typeof tabs)[0]) => {
    const connDbType = connections.find((c) => c.id === tab.connectionId)?.db_type;
    switch (tab.type) {
      case 'query':
        return (
          <QueryEditor
            connectionId={tab.connectionId}
            database={tab.database}
            tabKey={tab.key}
            initialContent={tab.sqlContent}
            dbType={connDbType}
          />
        );
      case 'table-data':
        return <DataGrid connectionId={tab.connectionId} database={tab.database} table={tab.table!} dbType={connDbType} />;
      case 'table-structure':
        return <TableStructure connectionId={tab.connectionId} database={tab.database} table={tab.table!} />;
      case 'view-definition':
      case 'function-definition':
      case 'procedure-definition':
        return (
          <ObjectDefinition
            connectionId={tab.connectionId}
            database={tab.database}
            objectName={tab.table!}
            objectType={tab.type.replace('-definition', '') as 'view' | 'function' | 'procedure'}
          />
        );
      case 'performance':
        return <PerformanceDashboard connectionId={tab.connectionId} database={tab.database} />;
      case 'data-chart':
        return <DataChartView connectionId={tab.connectionId} database={tab.database} table={tab.table!} />;
      case 'er-diagram':
        return <ERDiagram connectionId={tab.connectionId} database={tab.database} schema={tab.table} />;
      case 'redis-viewer':
        return <RedisViewer connectionId={tab.connectionId} database={tab.database} keyName={tab.table!} />;
      case 'mongo-viewer':
        return <MongoViewer connectionId={tab.connectionId} database={tab.database} collection={tab.table!} />;
      default:
        return null;
    }
  };

  return (
    <div className="flex h-screen flex-col bg-background text-foreground">
      <div className="flex flex-1 overflow-hidden">
        {/* Sidebar with fixed width */}
        <div className="shrink-0 border-r" style={{ width: sidebarWidth }}>
          <Sidebar />
        </div>

        {/* Resize handle */}
        <div
          className={cn(
            "w-1 shrink-0 cursor-col-resize transition-colors hover:bg-primary/20",
            isResizing && "bg-primary/30"
          )}
          onMouseDown={handleResizeStart}
        />

        {/* Main content */}
        <div className="flex flex-1 flex-col overflow-hidden">
          {tabs.length > 0 ? (
            <>
              {/* Tab Bar */}
              <div className="flex h-10 shrink-0 items-end overflow-x-auto border-b bg-muted/30 px-1">
                {tabs.map((tab) => (
                  <ContextMenu key={tab.key}>
                    <ContextMenuTrigger asChild>
                      <div
                        className={cn(
                          "group relative flex h-9 cursor-pointer items-center gap-2 rounded-t-md border-x border-t px-4 text-xs transition-colors",
                          tab.key === activeTabKey
                            ? "border-border bg-background text-foreground"
                            : "border-transparent bg-transparent text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                        )}
                        onClick={() => setActiveTab(tab.key)}
                      >
                        <span className="max-w-[150px] truncate">{tab.label}</span>
                        <button
                          className="ml-1 rounded-sm p-1 opacity-0 transition-opacity hover:bg-muted-foreground/20 group-hover:opacity-100"
                          onClick={(e) => { e.stopPropagation(); handleCloseTab(tab.key); }}
                        >
                          <X className="h-3 w-3" />
                        </button>
                      </div>
                    </ContextMenuTrigger>
                    <ContextMenuContent>
                      <ContextMenuItem onClick={() => handleCloseTab(tab.key)}>关闭当前标签页</ContextMenuItem>
                      <ContextMenuItem onClick={() => handleCloseAllTabs()}>关闭所有标签页</ContextMenuItem>
                      <ContextMenuItem onClick={() => handleCloseOtherTabs(tab.key)}>关闭其他标签页</ContextMenuItem>
                      <ContextMenuSeparator />
                      <ContextMenuItem onClick={() => handleCloseLeftTabs(tab.key)}>关闭左边的标签页</ContextMenuItem>
                      <ContextMenuItem onClick={() => handleCloseRightTabs(tab.key)}>关闭右边的标签页</ContextMenuItem>
                    </ContextMenuContent>
                  </ContextMenu>
                ))}
              </div>

              {/* Active Tab Content */}
              <div className="flex-1 overflow-hidden">
                {tabs.map((tab) => (
                  <div
                    key={tab.key}
                    className={cn("h-full", tab.key === activeTabKey ? "block" : "hidden")}
                  >
                    {renderTabContent(tab)}
                  </div>
                ))}
              </div>
            </>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center gap-4 text-muted-foreground">
              <Database className="h-14 w-14 opacity-20" />
              <p className="text-sm">在左侧选择一个连接开始使用</p>
              <p className="text-xs opacity-60">右键点击表名可查看数据或结构</p>
            </div>
          )}
        </div>
      </div>
      <StatusBar />
    </div>
  );
}
