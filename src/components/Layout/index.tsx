import { useState, useCallback, useRef, useEffect } from 'react';
import { useTabStore } from '@/stores/tabStore';
import { useConnectionStore } from '@/stores/connectionStore';
import QueryEditor from '../QueryEditor';
import DataGrid from '../DataGrid';
import TableStructure from '../TableStructure';
import ObjectDefinition from '../ObjectDefinition';
import PerformanceDashboard from '../PerformanceDashboard';
import DataChartView from '../DataChartView';
import Sidebar from '../Sidebar';
import StatusBar from '../StatusBar';
import { Database, X } from 'lucide-react';
import { cn } from '@/lib/utils';

const MIN_SIDEBAR_WIDTH = 200;
const MAX_SIDEBAR_WIDTH = 600;
const DEFAULT_SIDEBAR_WIDTH = 272;

export default function AppLayout() {
  const { tabs, activeTabKey, setActiveTab, removeTab } = useTabStore();
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
        return <DataGrid connectionId={tab.connectionId} database={tab.database} table={tab.table!} />;
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
                  <div
                    key={tab.key}
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
                      onClick={(e) => { e.stopPropagation(); removeTab(tab.key); }}
                    >
                      <X className="h-3 w-3" />
                    </button>
                  </div>
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
