import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { ConnectionConfig, DbType, DB_TYPE_LABELS, DEFAULT_PORTS } from '@/types/database';
import { useConnectionStore } from '@/stores/connectionStore';
import { CheckCircle, XCircle, Loader2 } from 'lucide-react';
import { cn } from '@/lib/utils';

interface Props {
  open: boolean;
  onClose: () => void;
  editConfig?: ConnectionConfig | null;
  readOnly?: boolean;
}

const dbTypes: DbType[] = ['mysql', 'postgresql', 'sqlite', 'sqlserver', 'mongodb', 'redis'];

const readableError = (error: unknown): string => {
  const value = error as { message?: string; remediation?: string };
  return [value?.message || String(error), value?.remediation]
    .filter(Boolean)
    .join(' ');
};

export default function ConnectionDialog({
  open,
  onClose,
  editConfig,
  readOnly = false,
}: Props) {
  const { t } = useTranslation();
  const { addConnection, updateConnection, testConnection } = useConnectionStore();

  const [form, setForm] = useState({
    name: '',
    db_type: 'mysql' as DbType,
    host: 'localhost',
    port: 3306,
    username: 'root',
    password: '',
    database: '',
    color: '#00758F',
  });
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testResult, setTestResult] = useState<{ success: boolean; message: string } | null>(null);

  useEffect(() => {
    if (open) {
      setTestResult(null);
      if (editConfig) {
        setForm({
          name: editConfig.name,
          db_type: editConfig.db_type,
          host: editConfig.host,
          port: editConfig.port,
          username: editConfig.username,
          password: '',
          database: editConfig.database || '',
          color: editConfig.color || '#00758F',
        });
      } else {
        setForm({
          name: '',
          db_type: 'mysql',
          host: 'localhost',
          port: 3306,
          username: 'root',
          password: '',
          database: '',
          color: '#00758F',
        });
      }
    }
  }, [open, editConfig]);

  const handleDbTypeChange = (dbType: DbType) => {
    if (readOnly) return;
    setForm((prev) => ({
      ...prev,
      db_type: dbType,
      port: DEFAULT_PORTS[dbType],
      host: dbType === 'sqlite' ? '' : prev.host || 'localhost',
      username: dbType === 'sqlite' || dbType === 'redis' ? '' : prev.username,
      password: '',
    }));
  };

  const closeDialog = () => {
    setForm((previous) => ({ ...previous, password: '' }));
    setTestResult(null);
    onClose();
  };

  const handleTest = async () => {
    if (readOnly || !form.name) return;
    setTesting(true);
    setTestResult(null);
    try {
      const config: ConnectionConfig = {
        id: editConfig?.id || crypto.randomUUID(),
        ...form,
        database: form.database || undefined,
        color: form.color || undefined,
      };
      const result = await testConnection(config);
      setTestResult(result);
    } catch (error: unknown) {
      setTestResult({ success: false, message: readableError(error) });
    } finally {
      setTesting(false);
    }
  };

  const handleSave = async () => {
    if (readOnly || !form.name) return;
    const config: ConnectionConfig = {
      id: editConfig?.id || crypto.randomUUID(),
      ...form,
      database: form.database || undefined,
      color: form.color || undefined,
    };
    setSaving(true);
    setTestResult(null);
    try {
      if (editConfig) {
        await updateConnection({
          ...config,
          revision: editConfig.revision,
          has_credential: editConfig.has_credential,
          mcp_enabled: editConfig.mcp_enabled,
        });
      } else {
        await addConnection(config);
      }
      closeDialog();
    } catch (error) {
      setTestResult({ success: false, message: readableError(error) });
    } finally {
      setSaving(false);
    }
  };

  const isSqlite = form.db_type === 'sqlite';
  const isRedis = form.db_type === 'redis';

  return (
    <Dialog open={open} onOpenChange={(v) => !v && closeDialog()}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>
            {readOnly
              ? t('connection.view')
              : editConfig
                ? t('connection.edit')
                : t('connection.new')}
          </DialogTitle>
          <DialogDescription>
            {readOnly
              ? t('connection.viewDescription')
              : editConfig
                ? t('connection.editDescription')
                : t('connection.newDescription')}
          </DialogDescription>
        </DialogHeader>

        <div className="mt-2 flex flex-col gap-5 py-2">
          {/* Connection Name */}
          <div className="grid grid-cols-4 items-center gap-4">
            <Label className="text-right">{t('connection.name')}</Label>
            <Input
              className="col-span-3"
              placeholder={t('connection.namePlaceholder')}
              value={form.name}
              onChange={(e) => setForm({ ...form, name: e.target.value })}
              readOnly={readOnly}
            />
          </div>

          {/* DB Type */}
          <div className="grid grid-cols-4 items-center gap-4">
            <Label className="text-right">{t('connection.type')}</Label>
            <Select
              value={form.db_type}
              onValueChange={(v) => handleDbTypeChange(v as DbType)}
              disabled={readOnly}
            >
              <SelectTrigger className="col-span-3">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {dbTypes.map((type) => (
                  <SelectItem key={type} value={type}>
                    {DB_TYPE_LABELS[type]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>

          {/* Host / File Path */}
          <div className="grid grid-cols-4 items-center gap-4">
            <Label className="text-right">
              {isSqlite ? t('connection.filePath') : t('connection.host')}
            </Label>
            <Input
              className="col-span-3"
              placeholder={isSqlite ? t('connection.filePathPlaceholder') : t('connection.hostPlaceholder')}
              value={form.host}
              onChange={(e) => setForm({ ...form, host: e.target.value })}
              readOnly={readOnly}
            />
          </div>

          {/* Port */}
          {!isSqlite && (
            <div className="grid grid-cols-4 items-center gap-4">
              <Label className="text-right">{t('connection.port')}</Label>
              <Input
                className="col-span-3"
                type="number"
                value={form.port}
                onChange={(e) => setForm({ ...form, port: parseInt(e.target.value) || 0 })}
                readOnly={readOnly}
              />
            </div>
          )}

          {/* Username */}
          {!isSqlite && !isRedis && (
            <div className="grid grid-cols-4 items-center gap-4">
              <Label className="text-right">{t('connection.username')}</Label>
              <Input
                className="col-span-3"
                placeholder={t('connection.usernamePlaceholder')}
                value={form.username}
                onChange={(e) => setForm({ ...form, username: e.target.value })}
                readOnly={readOnly}
              />
            </div>
          )}

          {/* Password */}
          {!isSqlite && (
            <div className="grid grid-cols-4 items-center gap-4">
              <Label className="text-right">{t('connection.password')}</Label>
              <Input
                className="col-span-3"
                type="password"
                placeholder={
                  readOnly
                    ? editConfig?.has_credential
                      ? t('connection.savedPasswordReadOnlyPlaceholder')
                      : t('connection.noSavedPasswordPlaceholder')
                    : editConfig?.has_credential
                      ? t('connection.savedPasswordPlaceholder')
                      : t('connection.passwordPlaceholder')
                }
                value={form.password}
                onChange={(e) => setForm({ ...form, password: e.target.value })}
                readOnly={readOnly}
              />
            </div>
          )}

          {/* Database */}
          {!isSqlite && (
            <div className="grid grid-cols-4 items-center gap-4">
              <Label className="text-right">{t('connection.database')}</Label>
              <Input
                className="col-span-3"
                placeholder={t('connection.databasePlaceholder')}
                value={form.database}
                onChange={(e) => setForm({ ...form, database: e.target.value })}
                readOnly={readOnly}
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="none"
                spellCheck={false}
              />
            </div>
          )}

          {/* Color */}
          <div className="grid grid-cols-4 items-center gap-4">
            <Label className="text-right">标识颜色</Label>
            <div className="col-span-3 flex items-center gap-3">
              <input
                type="color"
                value={form.color}
                onChange={(e) => setForm({ ...form, color: e.target.value })}
                disabled={readOnly}
                className={cn(
                  "h-9 w-12 rounded-md border border-input p-1",
                  readOnly ? "cursor-default" : "cursor-pointer"
                )}
              />
              <span className="text-xs text-muted-foreground">{form.color}</span>
            </div>
          </div>

          {/* Test Result */}
          {testResult && (
            <div
              className={cn(
                "flex items-center gap-2 rounded-md px-4 py-3 text-sm",
                testResult.success
                  ? "bg-emerald-50 text-emerald-700"
                  : "bg-red-50 text-red-700"
              )}
            >
              {testResult.success
                ? <CheckCircle className="h-4 w-4 shrink-0" />
                : <XCircle className="h-4 w-4 shrink-0" />
              }
              <span>{testResult.message}</span>
            </div>
          )}
        </div>

        <DialogFooter className="mt-4 gap-2">
          <Button variant="outline" onClick={closeDialog}>
            {readOnly ? t('connection.close') : t('connection.cancel')}
          </Button>
          {!readOnly && (
            <>
              <Button variant="secondary" onClick={handleTest} disabled={testing || !form.name}>
                {testing && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
                {t('connection.test')}
              </Button>
              <Button
                onClick={() => void handleSave()}
                disabled={!form.name || saving}
              >
                {saving && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
                {t('connection.save')}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
