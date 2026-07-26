import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Button } from '@/components/ui/button';
import { Loader2 } from 'lucide-react';
import { notify } from '@/stores/notificationStore';
import { useConnectionStore } from '@/stores/connectionStore';
import { quoteClickHouseIdentifier } from '@/lib/sqlIdentifier';

interface Props {
  connectionId: string;
  database: string;
  onSuccess: () => void;
}

export default function CreateUserForm({ connectionId, database, onSuccess }: Props) {
  const { t } = useTranslation();
  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [loading, setLoading] = useState(false);

  const connections = useConnectionStore((s) => s.connections);
  const conn = connections.find((c) => c.id === connectionId);
  const dbType = conn?.db_type;

  const handleSubmit = async () => {
    if (!username.trim() || !password) return;
    setLoading(true);
    try {
      let sql: string;
      if (dbType === 'postgresql') {
        sql = `CREATE USER "${username.trim()}" WITH PASSWORD '${password}'`;
      } else if (dbType === 'mysql') {
        sql = `CREATE USER '${username.trim()}'@'%' IDENTIFIED BY '${password}'`;
      } else if (dbType === 'sqlserver') {
        sql = `CREATE LOGIN [${username.trim()}] WITH PASSWORD = '${password}'`;
      } else if (dbType === 'clickhouse') {
        const escapedPassword = password.replace(/\\/g, '\\\\').replace(/'/g, "\\'");
        sql = `CREATE USER ${quoteClickHouseIdentifier(username.trim())} IDENTIFIED WITH sha256_password BY '${escapedPassword}'`;
      } else {
        sql = `CREATE USER "${username.trim()}" WITH PASSWORD '${password}'`;
      }

      await invoke('execute_query', { connectionId, database, sql });
      notify.success(t('create.success'), `${t('create.user')}: ${username.trim()}`);
      onSuccess();
    } catch (e) {
      notify.error(t('create.failed'), String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <div>
        <Label className="mb-1.5 block">{t('connection.username')}</Label>
        <Input
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          placeholder={t('connection.usernamePlaceholder')}
          autoFocus
        />
      </div>
      <div>
        <Label className="mb-1.5 block">{t('connection.password')}</Label>
        <Input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          placeholder={t('connection.passwordPlaceholder')}
        />
      </div>
      <Button onClick={handleSubmit} disabled={loading || !username.trim() || !password}>
        {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
        {t('common.confirm')}
      </Button>
    </div>
  );
}
