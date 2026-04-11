import { useTranslation } from 'react-i18next';
import {
  BarChart,
  Bar,
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

export default function PostgresDashboard({ data, loading }: Props) {
  const { t } = useTranslation();

  if (loading && !data) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!data) return null;

  const tupleData = [
    { name: 'Returned', value: data.tupReturned ?? 0 },
    { name: 'Fetched', value: data.tupFetched ?? 0 },
    { name: 'Inserted', value: data.tupInserted ?? 0 },
    { name: 'Updated', value: data.tupUpdated ?? 0 },
    { name: 'Deleted', value: data.tupDeleted ?? 0 },
  ];

  return (
    <div className="space-y-4">
      {/* Metric Cards Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
        <MetricCard
          label={t('performance.activeConnections')}
          value={data.activeConnections ?? 0}
        />
        <MetricCard
          label={`${t('performance.transactions')} (Commit)`}
          value={(data.xactCommit ?? 0).toLocaleString()}
        />
        <MetricCard
          label={`${t('performance.transactions')} (Rollback)`}
          value={(data.xactRollback ?? 0).toLocaleString()}
        />
        <MetricCard
          label={t('performance.cacheHitRate')}
          value={`${data.cacheHitRatio ?? 0}%`}
        />
        <MetricCard
          label={t('performance.deadTuples')}
          value={data.deadlocks ?? 0}
        />
      </div>

      {/* Secondary metrics */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
        <MetricCard
          label="Backends"
          value={data.numbackends ?? 0}
        />
        <MetricCard
          label={t('performance.tempFiles')}
          value={data.tempFiles ?? 0}
        />
        <MetricCard
          label="Temp Bytes"
          value={
            data.tempBytes
              ? data.tempBytes > 1024 * 1024
                ? `${(data.tempBytes / (1024 * 1024)).toFixed(1)} MB`
                : `${(data.tempBytes / 1024).toFixed(1)} KB`
              : '0'
          }
        />
      </div>

      {/* Tuple Operations Chart */}
      <div className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 text-sm font-medium">{t('performance.tuples')}</h3>
        <ChartContainer className="h-[200px]">
          <ResponsiveContainer width="100%" height={200}>
            <BarChart data={tupleData}>
              <CartesianGrid strokeDasharray="3 3" className="stroke-border" />
              <XAxis
                dataKey="name"
                className="text-xs"
                tick={{ fill: 'hsl(var(--muted-foreground))', fontSize: 11 }}
              />
              <YAxis
                className="text-xs"
                tick={{ fill: 'hsl(var(--muted-foreground))', fontSize: 11 }}
              />
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
              <Bar
                dataKey="value"
                name="Tuples"
                fill="hsl(210, 80%, 55%)"
                radius={[4, 4, 0, 0]}
              />
            </BarChart>
          </ResponsiveContainer>
        </ChartContainer>
      </div>
    </div>
  );
}
