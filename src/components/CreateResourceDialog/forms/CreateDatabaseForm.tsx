import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { Button } from '@/components/ui/button';
import { Loader2 } from 'lucide-react';
import { notify } from '@/stores/notificationStore';

interface Props {
  connectionId: string;
  onSuccess: () => void;
}

export default function CreateDatabaseForm({ connectionId, onSuccess }: Props) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [loading, setLoading] = useState(false);

  const handleSubmit = async () => {
    if (!name.trim()) return;
    setLoading(true);
    try {
      const sql = `CREATE DATABASE "${name.trim()}"`;
      await invoke('execute_query', { connectionId, database: '', sql });
      notify.success(t('create.success'), `CREATE DATABASE "${name.trim()}"`);
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
        <Label className="mb-1.5 block">{t('create.name')}</Label>
        <Input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t('create.database')}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="none"
          spellCheck={false}
          autoFocus
        />
      </div>
      <Button onClick={handleSubmit} disabled={loading || !name.trim()}>
        {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
        {t('common.confirm')}
      </Button>
    </div>
  );
}
