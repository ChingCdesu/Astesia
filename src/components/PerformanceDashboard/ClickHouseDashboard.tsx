import { Loader2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import MetricCard from './MetricCard';

interface Props {
  data: ClickHouseMetrics | null;
  loading: boolean;
}

interface ClickHouseMetrics {
  activeQueries: number;
  activeMerges: number;
  activeMutations: number;
  connections: number;
  memoryUsage: number;
  totalQueries: number;
  failedQueries: number;
  selectQueries: number;
  insertQueries: number;
  selectedRows: number;
  insertedRows: number;
  selectedBytes: number;
  insertedBytes: number;
  uptime: number;
  databaseCount: number;
  tableCount: number;
}

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  return [
    days > 0 ? `${days}d` : '',
    hours > 0 ? `${hours}h` : '',
    `${minutes}m`,
  ].filter(Boolean).join(' ');
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
}

function formatCount(value: number): string {
  return Math.round(value || 0).toLocaleString();
}

export default function ClickHouseDashboard({ data, loading }: Props) {
  const { t } = useTranslation();

  if (loading && !data) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }
  if (!data) return null;

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-6">
        <MetricCard
          label={t('performance.activeQueries')}
          value={formatCount(data.activeQueries)}
        />
        <MetricCard
          label={t('performance.activeMerges')}
          value={formatCount(data.activeMerges)}
        />
        <MetricCard
          label={t('performance.activeMutations')}
          value={formatCount(data.activeMutations)}
        />
        <MetricCard
          label={t('performance.connections')}
          value={formatCount(data.connections)}
        />
        <MetricCard
          label={t('performance.memory')}
          value={formatBytes(data.memoryUsage || 0)}
        />
        <MetricCard
          label={t('performance.uptime')}
          value={formatUptime(data.uptime || 0)}
        />
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <MetricCard
          label={t('performance.queries')}
          value={formatCount(data.totalQueries)}
        />
        <MetricCard
          label={t('performance.failedQueries')}
          value={formatCount(data.failedQueries)}
        />
        <MetricCard
          label={t('performance.selectedRows')}
          value={formatCount(data.selectedRows)}
        />
        <MetricCard
          label={t('performance.insertedRows')}
          value={formatCount(data.insertedRows)}
        />
      </div>

      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
        <MetricCard
          label={t('performance.selectQueries')}
          value={formatCount(data.selectQueries)}
        />
        <MetricCard
          label={t('performance.insertQueries')}
          value={formatCount(data.insertQueries)}
        />
        <MetricCard
          label={t('performance.selectedBytes')}
          value={formatBytes(data.selectedBytes || 0)}
        />
        <MetricCard
          label={t('performance.insertedBytes')}
          value={formatBytes(data.insertedBytes || 0)}
        />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <MetricCard
          label={t('performance.databaseCount')}
          value={formatCount(data.databaseCount)}
        />
        <MetricCard
          label={t('performance.tableCount')}
          value={formatCount(data.tableCount)}
        />
      </div>
    </div>
  );
}
