import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter, DialogDescription,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Badge } from '@/components/ui/badge';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import {
  ConnectionConfig,
  DbType,
  DB_TYPE_COLORS,
  DB_TYPE_LABELS,
  DEFAULT_PORTS,
} from '@/types/database';
import { useConnectionStore } from '@/stores/connectionStore';
import { CheckCircle, XCircle, Loader2, X } from 'lucide-react';
import { cn } from '@/lib/utils';

interface Props {
  open: boolean;
  onClose: () => void;
  editConfig?: ConnectionConfig | null;
  readOnly?: boolean;
}

const dbTypes: DbType[] = [
  'mysql',
  'postgresql',
  'sqlite',
  'sqlserver',
  'clickhouse',
  'mongodb',
  'redis',
];

const readableError = (error: unknown): string => {
  const value = error as { message?: string; remediation?: string };
  return [value?.message || String(error), value?.remediation]
    .filter(Boolean)
    .join(' ');
};

const mergeTags = (currentTags: string[], value: string): string[] => {
  const tags = [...currentTags];
  const keys = new Set(tags.map((tag) => tag.toLocaleLowerCase()));
  const candidates = value
    .split(/[,，\n]/)
    .map((tag) => tag.trim())
    .filter(Boolean);
  for (const tag of candidates) {
    const key = tag.toLocaleLowerCase();
    if (!keys.has(key) && tags.length < 20) {
      tags.push(tag);
      keys.add(key);
    }
  }
  return tags;
};

export default function ConnectionDialog({
  open,
  onClose,
  editConfig,
  readOnly = false,
}: Props) {
  const { t } = useTranslation();
  const { connections, addConnection, updateConnection, testConnection } = useConnectionStore();

  const [form, setForm] = useState({
    name: '',
    db_type: 'mysql' as DbType,
    host: 'localhost',
    port: 3306,
    username: 'root',
    password: '',
    database: '',
    color: '#00758F',
    group_name: '',
    tags: [] as string[],
  });
  const [tagInput, setTagInput] = useState('');
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [testResult, setTestResult] = useState<{ success: boolean; message: string } | null>(null);

  useEffect(() => {
    if (open) {
      setTestResult(null);
      setTagInput('');
      if (editConfig) {
        setForm({
          name: editConfig.name,
          db_type: editConfig.db_type,
          host: editConfig.host,
          port: editConfig.port,
          username: editConfig.username,
          password: '',
          database: editConfig.database || '',
          color: editConfig.color || DB_TYPE_COLORS[editConfig.db_type],
          group_name: editConfig.group_name || '',
          tags: editConfig.tags || [],
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
          group_name: '',
          tags: [],
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
      username: dbType === 'sqlite' || dbType === 'redis'
        ? ''
        : dbType === 'clickhouse'
          ? 'default'
          : prev.username,
      database: dbType === 'clickhouse' && !prev.database ? 'default' : prev.database,
      color: DB_TYPE_COLORS[dbType],
      password: '',
    }));
  };

  const closeDialog = () => {
    setForm((previous) => ({ ...previous, password: '' }));
    setTagInput('');
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
        group_name: form.group_name.trim() || undefined,
        tags: form.tags,
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
    const tags = mergeTags(form.tags, tagInput);
    const config: ConnectionConfig = {
      id: editConfig?.id || crypto.randomUUID(),
      ...form,
      database: form.database || undefined,
      color: form.color || undefined,
      group_name: form.group_name.trim() || undefined,
      tags,
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
  const isClickHouse = form.db_type === 'clickhouse';
  const existingGroups = useMemo(
    () => Array.from(new Set(
      connections
        .map((connection) => connection.group_name?.trim())
        .filter((group): group is string => Boolean(group))
    )).sort((left, right) => left.localeCompare(right)),
    [connections]
  );

  const addTags = (value: string) => {
    if (readOnly) return;
    setForm((previous) => ({ ...previous, tags: mergeTags(previous.tags, value) }));
    setTagInput('');
  };

  const removeTag = (tag: string) => {
    if (readOnly) return;
    setForm((previous) => ({
      ...previous,
      tags: previous.tags.filter((item) => item !== tag),
    }));
  };

  return (
    <Dialog open={open} onOpenChange={(v) => !v && closeDialog()}>
      <DialogContent className="max-h-[88vh] overflow-y-auto sm:max-w-[540px]">
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

          {/* Group */}
          <div className="grid grid-cols-4 items-center gap-4">
            <Label className="text-right">{t('connection.group')}</Label>
            <div className="col-span-3">
              <Input
                list="connection-group-options"
                placeholder={t('connection.groupPlaceholder')}
                value={form.group_name}
                onChange={(event) => setForm({ ...form, group_name: event.target.value })}
                readOnly={readOnly}
                maxLength={128}
              />
              <datalist id="connection-group-options">
                {existingGroups.map((group) => (
                  <option key={group} value={group} />
                ))}
              </datalist>
            </div>
          </div>

          {/* Tags */}
          <div className="grid grid-cols-4 items-start gap-4">
            <Label className="pt-2 text-right">{t('connection.tags')}</Label>
            <div className="col-span-3 space-y-2">
              {!readOnly && (
                <Input
                  placeholder={t('connection.tagsPlaceholder')}
                  value={tagInput}
                  onChange={(event) => setTagInput(event.target.value)}
                  onBlur={() => addTags(tagInput)}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' || event.key === ',' || event.key === '，') {
                      event.preventDefault();
                      addTags(tagInput);
                    }
                  }}
                  disabled={form.tags.length >= 20}
                  maxLength={64}
                />
              )}
              <div className="flex min-h-6 flex-wrap gap-1.5">
                {form.tags.length === 0 ? (
                  <span className="text-xs text-muted-foreground">
                    {t('connection.noTags')}
                  </span>
                ) : form.tags.map((tag) => (
                  <Badge key={tag} variant="secondary" className="gap-1 pr-1">
                    {tag}
                    {!readOnly && (
                      <button
                        type="button"
                        className="rounded-sm p-0.5 hover:bg-foreground/10"
                        aria-label={t('connection.removeTag', { tag })}
                        onClick={() => removeTag(tag)}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    )}
                  </Badge>
                ))}
              </div>
              {!readOnly && (
                <p className="text-[11px] text-muted-foreground">
                  {t('connection.tagsHint', { count: form.tags.length })}
                </p>
              )}
            </div>
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
            <>
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
              {isClickHouse && (
                <p className="-mt-3 pl-[calc(25%+1rem)] text-[11px] text-muted-foreground">
                  {t('connection.clickhouseHttpHint')}
                </p>
              )}
            </>
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
