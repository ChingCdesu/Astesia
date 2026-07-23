import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  Loader2,
  Play,
  RotateCw,
  Server,
  Square,
} from 'lucide-react';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { cn } from '@/lib/utils';
import { notify } from '@/stores/notificationStore';
import {
  type McpServiceState,
  useMcpHelperStore,
} from '@/stores/mcpHelperStore';

function statusDotClass(state: McpServiceState): string {
  switch (state) {
    case 'running':
      return 'bg-emerald-500';
    case 'starting':
    case 'stopping':
      return 'animate-pulse bg-amber-500';
    case 'error':
      return 'bg-destructive';
    default:
      return 'bg-zinc-400';
  }
}

function statusBadgeVariant(
  state: McpServiceState,
): 'success' | 'warning' | 'destructive' | 'secondary' {
  switch (state) {
    case 'running':
      return 'success';
    case 'starting':
    case 'stopping':
      return 'warning';
    case 'error':
      return 'destructive';
    default:
      return 'secondary';
  }
}

function formatStartedAt(value: string | null): string {
  if (!value) return '—';
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function createClientConfig(endpoint: string, authToken: string): string {
  return JSON.stringify({
    type: 'streamable-http',
    url: endpoint,
    headers: {
      Authorization: `Bearer ${authToken}`,
    },
  }, null, 2);
}

export default function McpHelper() {
  const { t } = useTranslation();
  const {
    status,
    port,
    authToken,
    operation,
    setPort,
    rotateAuthToken,
    refreshStatus,
    startService,
    stopService,
    restartService,
  } = useMcpHelperStore();
  const [open, setOpen] = useState(false);
  const [portValue, setPortValue] = useState(String(port));
  const [showToken, setShowToken] = useState(false);

  useEffect(() => {
    void refreshStatus();
    const timer = window.setInterval(() => {
      void refreshStatus();
    }, 2000);
    return () => window.clearInterval(timer);
  }, [refreshStatus]);

  const configuredEndpoint = status.endpoint ?? `http://127.0.0.1:${port}/mcp`;
  const displayedClientConfig = useMemo(
    () => createClientConfig(
      configuredEndpoint,
      showToken ? authToken : '<hidden>',
    ),
    [authToken, configuredEndpoint, showToken],
  );

  const commitPort = (): number | null => {
    const nextPort = Number(portValue);
    if (!Number.isInteger(nextPort) || nextPort < 1024 || nextPort > 65535) {
      setPortValue(String(port));
      notify.warning(t('mcpHelper.title'), t('mcpHelper.invalidPort'));
      return null;
    }
    setPort(nextPort);
    return nextPort;
  };

  const handleCopyConfig = async () => {
    const nextPort = commitPort();
    if (nextPort === null) return;
    const endpoint = status.endpoint ?? `http://127.0.0.1:${nextPort}/mcp`;
    try {
      await navigator.clipboard.writeText(createClientConfig(endpoint, authToken));
      notify.success(t('mcpHelper.title'), t('mcpHelper.copied'));
    } catch (error) {
      notify.error(t('mcpHelper.title'), `${t('mcpHelper.copyFailed')}: ${String(error)}`);
    }
  };

  const handleStart = () => {
    if (commitPort() !== null) void startService();
  };

  const handleRestart = () => {
    if (commitPort() !== null) void restartService();
  };

  const handleRotateToken = async () => {
    const restartRequired = status.pid !== null || status.state === 'running';
    rotateAuthToken();
    setShowToken(false);
    if (restartRequired) {
      await restartService();
    }
    notify.success(t('mcpHelper.title'), t('mcpHelper.tokenRotated'));
  };

  const isBusy = operation !== null;
  const canStart = status.available
    && status.pid === null
    && status.state !== 'running'
    && status.state !== 'starting'
    && status.state !== 'stopping';
  const canStop = status.pid !== null || status.state === 'running';
  const canRestart = status.available && (status.pid !== null || status.state === 'running');

  return (
    <>
      <Button
        variant="ghost"
        size="sm"
        className="h-7 gap-1.5 px-2 text-xs"
        title={t('mcpHelper.open')}
        aria-label={t('mcpHelper.open')}
        onClick={() => {
          setPortValue(String(port));
          setOpen(true);
        }}
      >
        <Server className="h-3.5 w-3.5" />
        <span>MCP</span>
        <span className={cn('h-1.5 w-1.5 rounded-full', statusDotClass(status.state))} />
      </Button>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="max-h-[85vh] max-w-2xl overflow-y-auto">
          <DialogHeader>
            <DialogTitle>{t('mcpHelper.title')}</DialogTitle>
            <DialogDescription>{t('mcpHelper.description')}</DialogDescription>
          </DialogHeader>

          <section className="space-y-3 rounded-md border p-4">
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-sm font-medium">{t('mcpHelper.serverStatus')}</h3>
              <Badge variant={statusBadgeVariant(status.state)}>
                {t(`mcpHelper.states.${status.state}`)}
              </Badge>
            </div>

            <dl className="grid grid-cols-[8rem_minmax(0,1fr)] gap-x-4 gap-y-2 text-sm">
              <dt className="text-muted-foreground">{t('mcpHelper.availability')}</dt>
              <dd>
                <Badge variant={status.available ? 'success' : 'destructive'}>
                  {status.available ? t('mcpHelper.available') : t('mcpHelper.unavailable')}
                </Badge>
              </dd>

              <dt className="text-muted-foreground">{t('mcpHelper.pid')}</dt>
              <dd className="font-mono">{status.pid ?? '—'}</dd>

              <dt className="text-muted-foreground">{t('mcpHelper.endpoint')}</dt>
              <dd className="break-all font-mono text-xs">{status.endpoint ?? '—'}</dd>

              <dt className="text-muted-foreground">{t('mcpHelper.transport')}</dt>
              <dd>{t('mcpHelper.streamableHttp')}</dd>

              <dt className="text-muted-foreground">{t('mcpHelper.version')}</dt>
              <dd className="font-mono">{status.version ?? '—'}</dd>

              <dt className="text-muted-foreground">{t('mcpHelper.startedAt')}</dt>
              <dd>{formatStartedAt(status.started_at)}</dd>

              <dt className="text-muted-foreground">{t('mcpHelper.binaryPath')}</dt>
              <dd className="break-all font-mono text-xs" title={status.binary_path ?? undefined}>
                {status.binary_path ?? '—'}
              </dd>
            </dl>

            {status.last_error && (
              <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3">
                <div className="mb-1 text-xs font-medium text-destructive">
                  {t('mcpHelper.lastError')}
                </div>
                <div className="break-words font-mono text-xs text-destructive">
                  {status.last_error}
                </div>
              </div>
            )}
          </section>

          <section className="space-y-3 rounded-md border p-4">
            <div className="space-y-1.5">
              <Label htmlFor="mcp-helper-port">{t('mcpHelper.port')}</Label>
              <Input
                id="mcp-helper-port"
                type="number"
                min={1024}
                max={65535}
                value={portValue}
                disabled={isBusy}
                onChange={(event) => setPortValue(event.target.value)}
                onBlur={() => { commitPort(); }}
              />
              <p className="text-xs text-muted-foreground">{t('mcpHelper.portHint')}</p>
            </div>

            <div className="flex flex-wrap gap-2">
              <Button size="sm" disabled={isBusy || !canStart} onClick={handleStart}>
                {operation === 'start'
                  ? <Loader2 className="animate-spin" />
                  : <Play />}
                {operation === 'start' ? t('mcpHelper.starting') : t('mcpHelper.start')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={isBusy || !canStop}
                onClick={() => void stopService()}
              >
                {operation === 'stop'
                  ? <Loader2 className="animate-spin" />
                  : <Square />}
                {operation === 'stop' ? t('mcpHelper.stopping') : t('mcpHelper.stop')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={isBusy || !canRestart}
                onClick={handleRestart}
              >
                {operation === 'restart'
                  ? <Loader2 className="animate-spin" />
                  : <RotateCw />}
                {operation === 'restart' ? t('mcpHelper.restarting') : t('mcpHelper.restart')}
              </Button>
            </div>
          </section>

          <section className="space-y-3 rounded-md border p-4">
            <div>
              <h3 className="text-sm font-medium">{t('mcpHelper.clientConfig')}</h3>
              <p className="mt-1 text-xs text-muted-foreground">
                {t('mcpHelper.clientConfigDescription')}
              </p>
            </div>
            <pre className="max-h-48 overflow-auto rounded-md bg-muted p-3 text-xs">
              <code>{displayedClientConfig}</code>
            </pre>
            <div className="flex flex-wrap items-center justify-between gap-3">
              <p className="text-xs text-amber-700 dark:text-amber-400">
                {t('mcpHelper.securityNote')}
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => setShowToken((visible) => !visible)}
                >
                  {showToken ? <EyeOff /> : <Eye />}
                  {showToken ? t('mcpHelper.hideToken') : t('mcpHelper.showToken')}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  disabled={isBusy}
                  onClick={() => void handleRotateToken()}
                >
                  <KeyRound />
                  {t('mcpHelper.rotateToken')}
                </Button>
                <Button size="sm" variant="outline" onClick={() => void handleCopyConfig()}>
                  <Copy />
                  {t('mcpHelper.copyConfig')}
                </Button>
              </div>
            </div>
          </section>
        </DialogContent>
      </Dialog>
    </>
  );
}
