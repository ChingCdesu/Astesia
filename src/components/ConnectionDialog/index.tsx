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
}

const dbTypes: DbType[] = ['mysql', 'postgresql', 'sqlite', 'sqlserver', 'mongodb', 'redis'];

export default function ConnectionDialog({ open, onClose, editConfig }: Props) {
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
          password: editConfig.password,
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
    setForm((prev) => ({
      ...prev,
      db_type: dbType,
      port: DEFAULT_PORTS[dbType],
      host: dbType === 'sqlite' ? '' : prev.host || 'localhost',
      username: dbType === 'sqlite' || dbType === 'redis' ? '' : prev.username,
    }));
  };

  const handleTest = async () => {
    if (!form.name) return;
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
    } catch (e: any) {
      setTestResult({ success: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  };

  const handleSave = () => {
    if (!form.name) return;
    const config: ConnectionConfig = {
      id: editConfig?.id || crypto.randomUUID(),
      ...form,
      database: form.database || undefined,
      color: form.color || undefined,
    };
    if (editConfig) {
      updateConnection(config);
    } else {
      addConnection(config);
    }
    onClose();
  };

  const isSqlite = form.db_type === 'sqlite';
  const isRedis = form.db_type === 'redis';

  return (
    <Dialog open={open} onOpenChange={(v) => !v && onClose()}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <DialogTitle>
            {editConfig ? t('connection.edit') : t('connection.new')}
          </DialogTitle>
          <DialogDescription>
            {editConfig ? '修改数据库连接配置' : '配置一个新的数据库连接'}
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
            />
          </div>

          {/* DB Type */}
          <div className="grid grid-cols-4 items-center gap-4">
            <Label className="text-right">{t('connection.type')}</Label>
            <Select value={form.db_type} onValueChange={(v) => handleDbTypeChange(v as DbType)}>
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
                placeholder={t('connection.passwordPlaceholder')}
                value={form.password}
                onChange={(e) => setForm({ ...form, password: e.target.value })}
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
                className="h-9 w-12 cursor-pointer rounded-md border border-input p-1"
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
          <Button variant="outline" onClick={onClose}>
            {t('connection.cancel')}
          </Button>
          <Button variant="secondary" onClick={handleTest} disabled={testing || !form.name}>
            {testing && <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />}
            {t('connection.test')}
          </Button>
          <Button onClick={handleSave} disabled={!form.name}>
            {t('connection.save')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
