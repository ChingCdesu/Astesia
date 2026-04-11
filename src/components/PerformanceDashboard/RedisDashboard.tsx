import { useTranslation } from 'react-i18next';
import {
  RadialBarChart,
  RadialBar,
  ResponsiveContainer,
  PolarAngleAxis,
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

export default function RedisDashboard({ data, loading }: Props) {
  const { t } = useTranslation();

  if (loading && !data) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!data) return null;

  const hitRate = data.hitRate ?? 0;
  const gaugeData = [{ name: 'Hit Rate', value: hitRate, fill: 'hsl(150, 60%, 45%)' }];

  return (
    <div className="space-y-4">
      {/* Metric Cards Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
        <MetricCard
          label={t('performance.connections')}
          value={data.connectedClients ?? 0}
        />
        <MetricCard
          label={t('performance.memory')}
          value={data.usedMemoryHuman || '0B'}
        />
        <MetricCard
          label={t('performance.memoryPeak')}
          value={data.usedMemoryPeakHuman || '0B'}
        />
        <MetricCard
          label={t('performance.commands')}
          value={(data.totalCommandsProcessed ?? 0).toLocaleString()}
        />
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <MetricCard
          label={t('performance.hitRate')}
          value={`${hitRate}%`}
        />
        <MetricCard
          label={t('performance.evictedKeys')}
          value={data.evictedKeys ?? 0}
        />
        <MetricCard
          label={t('performance.uptime')}
          value={formatUptime(data.uptimeInSeconds ?? 0)}
        />
        <MetricCard
          label="Redis Version"
          value={data.redisVersion || '-'}
        />
      </div>

      {/* Keyspace Hit Rate Gauge */}
      <div className="rounded-lg border bg-card p-4">
        <h3 className="mb-3 text-sm font-medium">
          Keyspace {t('performance.hitRate')}
        </h3>
        <ChartContainer className="h-[200px]">
          <ResponsiveContainer width="100%" height={200}>
            <RadialBarChart
              cx="50%"
              cy="50%"
              innerRadius="60%"
              outerRadius="90%"
              barSize={20}
              data={gaugeData}
              startAngle={180}
              endAngle={0}
            >
              <PolarAngleAxis
                type="number"
                domain={[0, 100]}
                angleAxisId={0}
                tick={false}
              />
              <RadialBar
                background={{ fill: 'hsl(var(--muted))' }}
                dataKey="value"
                angleAxisId={0}
                cornerRadius={10}
              />
              <text
                x="50%"
                y="50%"
                textAnchor="middle"
                dominantBaseline="middle"
                className="fill-foreground text-2xl font-bold"
              >
                {hitRate}%
              </text>
            </RadialBarChart>
          </ResponsiveContainer>
        </ChartContainer>
      </div>

      {/* Keyspace Stats */}
      <div className="grid grid-cols-2 gap-3">
        <MetricCard
          label="Keyspace Hits"
          value={(data.keyspaceHits ?? 0).toLocaleString()}
        />
        <MetricCard
          label="Keyspace Misses"
          value={(data.keyspaceMisses ?? 0).toLocaleString()}
        />
      </div>
    </div>
  );
}
