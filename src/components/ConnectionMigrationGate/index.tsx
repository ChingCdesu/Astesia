import { useTranslation } from 'react-i18next';
import { AlertTriangle, Database, Loader2, ShieldCheck } from 'lucide-react';

import { Button } from '@/components/ui/button';

export interface MigrationFailure {
  message: string;
  remediation?: string;
  code?: string;
}

interface Props {
  connectionCount: number;
  canMigrate: boolean;
  migrating: boolean;
  failure: MigrationFailure | null;
  onMigrate: () => void;
}

export default function ConnectionMigrationGate({
  connectionCount,
  canMigrate,
  migrating,
  failure,
  onMigrate,
}: Props) {
  const { t } = useTranslation();

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-background p-6">
      <div className="w-full max-w-xl rounded-xl border bg-card p-7 shadow-xl">
        <div className="mb-6 flex items-start gap-4">
          <div className="rounded-lg bg-primary/10 p-3 text-primary">
            <ShieldCheck className="h-6 w-6" />
          </div>
          <div>
            <h1 className="text-xl font-semibold">{t('migration.title')}</h1>
            <p className="mt-2 text-sm leading-6 text-muted-foreground">
              {t('migration.description', { count: connectionCount })}
            </p>
          </div>
        </div>

        <div className="mb-6 space-y-3 rounded-lg border bg-muted/40 p-4 text-sm">
          <div className="flex gap-3">
            <Database className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
            <span>{t('migration.metadata')}</span>
          </div>
          <div className="flex gap-3">
            <ShieldCheck className="mt-0.5 h-4 w-4 shrink-0 text-primary" />
            <span>{t('migration.credentials')}</span>
          </div>
        </div>

        {failure && (
          <div className="mb-6 rounded-lg border border-destructive/40 bg-destructive/10 p-4">
            <div className="flex gap-3">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
              <div className="space-y-1 text-sm">
                <p className="font-medium text-destructive">{failure.message}</p>
                {failure.remediation && (
                  <p className="text-muted-foreground">{failure.remediation}</p>
                )}
                {failure.code && (
                  <p className="font-mono text-xs text-muted-foreground">
                    {t('migration.errorCode', { code: failure.code })}
                  </p>
                )}
              </div>
            </div>
          </div>
        )}

        <p className="mb-5 text-xs leading-5 text-muted-foreground">
          {t('migration.required')}
        </p>
        <Button
          className="w-full"
          size="lg"
          disabled={migrating || !canMigrate}
          onClick={onMigrate}
        >
          {migrating && <Loader2 className="h-4 w-4 animate-spin" />}
          {migrating ? t('migration.migrating') : t('migration.start')}
        </Button>
      </div>
    </div>
  );
}
