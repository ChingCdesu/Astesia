import { useTranslation } from 'react-i18next';
import { Loader2 } from 'lucide-react';
import MetricCard from './MetricCard';

interface Props {
  data: any;
  loading: boolean;
}

export default function SQLServerDashboard({ data, loading }: Props) {
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
      {/* Metric Cards Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
        <MetricCard
          label={t('performance.batchRequests')}
          value={(data.batchRequestsPerSec ?? 0).toLocaleString()}
        />
        <MetricCard
          label={t('performance.cacheHitRate')}
          value={`${data.bufferCacheHitRatio ?? 0}%`}
        />
        <MetricCard
          label={t('performance.sessions')}
          value={data.activeSessions ?? 0}
        />
        <MetricCard
          label={t('performance.memory') + ' Grants'}
          value={data.memoryGrants ?? 0}
        />
      </div>

      {/* Page Life Expectancy */}
      <div className="grid grid-cols-2 gap-3">
        <MetricCard
          label="Page Life Expectancy"
          value={data.pageLifeExpectancy ?? 0}
          unit="sec"
        />
      </div>
    </div>
  );
}
