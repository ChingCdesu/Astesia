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

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export default function MongoDBDashboard({ data, loading }: Props) {
  const { t } = useTranslation();

  if (loading && !data) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!data) return null;

  const opCounterData = [
    { name: 'Insert', value: data.opInsert ?? 0 },
    { name: 'Query', value: data.opQuery ?? 0 },
    { name: 'Update', value: data.opUpdate ?? 0 },
    { name: 'Delete', value: data.opDelete ?? 0 },
  ];

  return (
    <div className="space-y-4">
      {/* Metric Cards Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
        <MetricCard
          label={t('performance.connections')}
          value={data.connections ?? 0}
        />
        <MetricCard
          label={`${t('performance.memory')} (Resident)`}
          value={data.memResident ? `${data.memResident} MB` : '0 MB'}
        />
        <MetricCard
          label={`${t('performance.memory')} (Virtual)`}
          value={data.memVirtual ? `${data.memVirtual} MB` : '0 MB'}
        />
        <MetricCard
          label={t('performance.bytesIO')}
          value={formatBytes((data.networkBytesIn ?? 0) + (data.networkBytesOut ?? 0))}
        />
      </div>

      {/* OpCounters Chart */}
      <div className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 text-sm font-medium">{t('performance.opCounters')}</h3>
        <ChartContainer className="h-[200px]">
          <ResponsiveContainer width="100%" height={200}>
            <BarChart data={opCounterData}>
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
                name="Operations"
                fill="hsl(150, 60%, 45%)"
                radius={[4, 4, 0, 0]}
              />
            </BarChart>
          </ResponsiveContainer>
        </ChartContainer>
      </div>
    </div>
  );
}
