import { useTranslation } from 'react-i18next';
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  ResponsiveContainer,
  Legend,
} from 'recharts';
import { Loader2 } from 'lucide-react';
import MetricCard from './MetricCard';
import { ChartContainer } from '@/components/ui/chart';

interface Props {
  data: any;
  loading: boolean;
}

function formatUptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const h = Math.floor((seconds % 86400) / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const parts: string[] = [];
  if (d > 0) parts.push(`${d}d`);
  if (h > 0) parts.push(`${h}h`);
  parts.push(`${m}m`);
  return parts.join(' ');
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export default function MySQLDashboard({ data, loading }: Props) {
  const { t } = useTranslation();

  if (loading && !data) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!data) return null;

  const qpsData = [
    {
      name: 'QPS',
      SELECT: data.comSelect || 0,
      INSERT: data.comInsert || 0,
      UPDATE: data.comUpdate || 0,
      DELETE: data.comDelete || 0,
    },
  ];

  return (
    <div className="space-y-4">
      {/* Metric Cards Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
        <MetricCard
          label={t('performance.connections')}
          value={data.threadsConnected ?? data.connections ?? 0}
        />
        <MetricCard
          label={t('performance.threadsRunning')}
          value={data.threadsRunning ?? 0}
        />
        <MetricCard
          label={t('performance.queries')}
          value={(data.queries ?? 0).toLocaleString()}
        />
        <MetricCard
          label={t('performance.slowQueries')}
          value={data.slowQueries ?? 0}
        />
        <MetricCard
          label={t('performance.cacheHitRate')}
          value={`${data.bufferPoolHitRate ?? 0}%`}
        />
        <MetricCard
          label={t('performance.uptime')}
          value={formatUptime(data.uptime ?? 0)}
        />
      </div>

      {/* Network IO */}
      <div className="grid grid-cols-2 gap-3">
        <MetricCard
          label={`${t('performance.bytesIO')} (Received)`}
          value={formatBytes(data.bytesReceived ?? 0)}
        />
        <MetricCard
          label={`${t('performance.bytesIO')} (Sent)`}
          value={formatBytes(data.bytesSent ?? 0)}
        />
      </div>

      {/* QPS Breakdown Chart */}
      <div className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 text-sm font-medium">
          {t('performance.qps')} - SELECT / INSERT / UPDATE / DELETE
        </h3>
        <ChartContainer className="h-[200px]">
          <ResponsiveContainer width="100%" height={200}>
            <AreaChart data={qpsData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis dataKey="name" className="text-xs" tick={{ fill: 'hsl(var(--muted-foreground))' }} />
              <YAxis className="text-xs" tick={{ fill: 'hsl(var(--muted-foreground))' }} />
              <Tooltip
                contentStyle={{
                  backgroundColor: 'hsl(var(--card))',
                  border: '1px solid hsl(var(--border))',
                  borderRadius: '6px',
                  fontSize: '12px',
                }}
                labelStyle={{ color: 'hsl(var(--foreground))' }}
              />
              <Legend wrapperStyle={{ fontSize: '12px' }} />
              <Area
                type="monotone"
                dataKey="SELECT"
                stackId="1"
                stroke="hsl(210, 80%, 55%)"
                fill="hsl(210, 80%, 55%)"
                fillOpacity={0.6}
              />
              <Area
                type="monotone"
                dataKey="INSERT"
                stackId="1"
                stroke="hsl(150, 60%, 45%)"
                fill="hsl(150, 60%, 45%)"
                fillOpacity={0.6}
              />
              <Area
                type="monotone"
                dataKey="UPDATE"
                stackId="1"
                stroke="hsl(40, 80%, 50%)"
                fill="hsl(40, 80%, 50%)"
                fillOpacity={0.6}
              />
              <Area
                type="monotone"
                dataKey="DELETE"
                stackId="1"
                stroke="hsl(0, 70%, 55%)"
                fill="hsl(0, 70%, 55%)"
                fillOpacity={0.6}
              />
            </AreaChart>
          </ResponsiveContainer>
        </ChartContainer>
      </div>
    </div>
  );
}
