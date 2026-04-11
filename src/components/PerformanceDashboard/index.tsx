import { useEffect, useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useConnectionStore } from '@/stores/connectionStore';
import { useTranslation } from 'react-i18next';
import { RefreshCw, Loader2 } from 'lucide-react';
import { Button } from '@/components/ui/button';
import MySQLDashboard from './MySQLDashboard';
import PostgresDashboard from './PostgresDashboard';
import SQLiteDashboard from './SQLiteDashboard';
import SQLServerDashboard from './SQLServerDashboard';
import MongoDBDashboard from './MongoDBDashboard';
import RedisDashboard from './RedisDashboard';

interface Props {
  connectionId: string;
  database: string;
}

export default function PerformanceDashboard({ connectionId, database }: Props) {
  const { t } = useTranslation();
  const connections = useConnectionStore((s) => s.connections);
  const connection = connections.find((c) => c.id === connectionId);
  const dbType = connection?.db_type;

  const [data, setData] = useState<any>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [interval, setInterval_] = useState(10);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const fetchMetrics = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<any>('get_performance_metrics', {
        connectionId,
        database,
      });
      setData(result);
    } catch (e: any) {
      setError(typeof e === 'string' ? e : e.message || '获取指标失败');
    } finally {
      setLoading(false);
    }
  }, [connectionId, database]);

  useEffect(() => {
    fetchMetrics();
  }, [fetchMetrics]);

  useEffect(() => {
    if (autoRefresh) {
      intervalRef.current = setInterval(fetchMetrics, interval * 1000);
    }
    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
    };
  }, [autoRefresh, interval, fetchMetrics]);

  const renderDashboard = () => {
    switch (dbType) {
      case 'mysql':
        return <MySQLDashboard data={data} loading={loading} />;
      case 'postgresql':
        return <PostgresDashboard data={data} loading={loading} />;
      case 'sqlite':
        return <SQLiteDashboard data={data} loading={loading} />;
      case 'sqlserver':
        return <SQLServerDashboard data={data} loading={loading} />;
      case 'mongodb':
        return <MongoDBDashboard data={data} loading={loading} />;
      case 'redis':
        return <RedisDashboard data={data} loading={loading} />;
      default:
        return <div className="p-4 text-muted-foreground">不支持的数据库类型</div>;
    }
  };

  return (
    <div className="flex h-full flex-col overflow-hidden">
      {/* Toolbar */}
      <div className="flex shrink-0 items-center gap-3 border-b px-4 py-2">
        <h2 className="text-sm font-semibold">{t('performance.title')}</h2>
        <div className="ml-auto flex items-center gap-2">
          {/* Auto-refresh toggle */}
          <label className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <input
              type="checkbox"
              checked={autoRefresh}
              onChange={(e) => setAutoRefresh(e.target.checked)}
              className="h-3.5 w-3.5 rounded border-border"
            />
            {t('performance.autoRefresh')}
          </label>

          {/* Interval selector */}
          {autoRefresh && (
            <select
              value={interval}
              onChange={(e) => setInterval_(Number(e.target.value))}
              className="h-7 rounded border bg-background px-2 text-xs"
            >
              <option value={5}>5s</option>
              <option value={10}>10s</option>
              <option value={30}>30s</option>
              <option value={60}>60s</option>
            </select>
          )}

          {/* Manual refresh */}
          <Button
            variant="outline"
            size="sm"
            onClick={fetchMetrics}
            disabled={loading}
            className="h-7 gap-1 px-2 text-xs"
          >
            {loading ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <RefreshCw className="h-3.5 w-3.5" />
            )}
            {t('performance.refresh')}
          </Button>
        </div>
      </div>

      {/* Error state */}
      {error && (
        <div className="mx-4 mt-2 rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      )}

      {/* Dashboard content */}
      <div className="flex-1 overflow-auto p-4">
        {renderDashboard()}
      </div>
    </div>
  );
}
