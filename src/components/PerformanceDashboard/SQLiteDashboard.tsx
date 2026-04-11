import { useTranslation } from 'react-i18next';
import { Loader2 } from 'lucide-react';
import MetricCard from './MetricCard';

interface Props {
  data: any;
  loading: boolean;
}

export default function SQLiteDashboard({ data, loading }: Props) {
  const { t } = useTranslation();

  if (loading && !data) {
    return (
      <div className="flex items-center justify-center py-20">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (!data) return null;

  const cacheSize = typeof data.cacheSize === 'string'
    ? data.cacheSize
    : String(data.cacheSize ?? 0);

  const pageCount = typeof data.pageCount === 'string'
    ? data.pageCount
    : String(data.pageCount ?? 0);

  const pageSize = typeof data.pageSize === 'string'
    ? data.pageSize
    : String(data.pageSize ?? 0);

  const journalMode = typeof data.journalMode === 'string'
    ? data.journalMode
    : String(data.journalMode ?? 'unknown');

  const walPages = typeof data.walPages === 'string'
    ? data.walPages
    : String(data.walPages ?? 0);

  // Calculate estimated DB size
  const pageSizeNum = parseInt(pageSize, 10) || 0;
  const pageCountNum = parseInt(pageCount, 10) || 0;
  const dbSizeBytes = pageSizeNum * pageCountNum;
  const dbSizeDisplay =
    dbSizeBytes > 1024 * 1024
      ? `${(dbSizeBytes / (1024 * 1024)).toFixed(1)} MB`
      : dbSizeBytes > 1024
        ? `${(dbSizeBytes / 1024).toFixed(1)} KB`
        : `${dbSizeBytes} B`;

  return (
    <div className="space-y-4">
      {/* Metric Cards Grid */}
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
        <MetricCard
          label={t('performance.cacheSize')}
          value={cacheSize}
          unit="pages"
        />
        <MetricCard
          label={t('performance.pageCount')}
          value={pageCount}
        />
        <MetricCard
          label={t('performance.pageSize')}
          value={pageSize}
          unit="bytes"
        />
        <MetricCard
          label={t('performance.journalMode')}
          value={journalMode.toUpperCase()}
        />
      </div>

      {/* Secondary info */}
      <div className="grid grid-cols-2 gap-3">
        <MetricCard
          label="WAL Pages"
          value={walPages}
        />
        <MetricCard
          label="DB Size (est.)"
          value={dbSizeDisplay}
        />
      </div>

      {/* Info note */}
      <div className="rounded-lg border bg-muted/30 p-4 text-xs text-muted-foreground">
        SQLite 是嵌入式数据库，性能监控指标有限。上述指标来自 PRAGMA 查询。
      </div>
    </div>
  );
}
