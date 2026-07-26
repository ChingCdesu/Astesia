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
  schema?: string;
  onSuccess: () => void;
  isProcedure?: boolean;
}

export default function CreateFunctionForm({ connectionId, database, schema, onSuccess, isProcedure = false }: Props) {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [language, setLanguage] = useState('plpgsql');
  const [body, setBody] = useState('');
  const [loading, setLoading] = useState(false);

  const connections = useConnectionStore((s) => s.connections);
  const conn = connections.find((c) => c.id === connectionId);
  const dbType = conn?.db_type;

  const handleSubmit = async () => {
    if (!name.trim() || !body.trim()) return;
    setLoading(true);
    try {
      let sql: string;
      const qualifiedName = schema ? `"${schema}"."${name.trim()}"` : `"${name.trim()}"`;

      if (dbType === 'postgresql') {
        if (isProcedure) {
          sql = `CREATE OR REPLACE PROCEDURE ${qualifiedName}()\nLANGUAGE ${language}\nAS $$\n${body.trim()}\n$$`;
        } else {
          sql = `CREATE OR REPLACE FUNCTION ${qualifiedName}()\nRETURNS void\nLANGUAGE ${language}\nAS $$\n${body.trim()}\n$$`;
        }
      } else if (dbType === 'mysql') {
        const keyword = isProcedure ? 'PROCEDURE' : 'FUNCTION';
        sql = `CREATE ${keyword} \`${name.trim()}\`()\nBEGIN\n${body.trim()}\nEND`;
      } else if (dbType === 'clickhouse' && !isProcedure) {
        sql = `CREATE FUNCTION ${quoteClickHouseIdentifier(name.trim())} AS ${body.trim()}`;
      } else {
        sql = `CREATE FUNCTION "${name.trim()}"()\nAS\nBEGIN\n${body.trim()}\nEND`;
      }

      await invoke('execute_query', { connectionId, database, sql });
      const resourceLabel = isProcedure ? t('create.procedure') : t('create.function');
      notify.success(t('create.success'), `${resourceLabel}: ${name.trim()}`);
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
          placeholder={isProcedure ? t('create.procedure') : t('create.function')}
          autoFocus
        />
      </div>
      {dbType === 'postgresql' && (
        <div>
          <Label className="mb-1.5 block">Language</Label>
          <select
            className="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
            value={language}
            onChange={(e) => setLanguage(e.target.value)}
          >
            <option value="plpgsql">PL/pgSQL</option>
            <option value="sql">SQL</option>
            <option value="plpython3u">PL/Python</option>
          </select>
        </div>
      )}
      <div>
        <Label className="mb-1.5 block">{t('create.definition')}</Label>
        <textarea
          className="flex min-h-[120px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring font-mono"
          value={body}
          onChange={(e) => setBody(e.target.value)}
          placeholder={
            dbType === 'clickhouse'
              ? '(x) -> x * 2'
              : '-- function body'
          }
        />
      </div>
      <Button onClick={handleSubmit} disabled={loading || !name.trim() || !body.trim()}>
        {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
        {t('common.confirm')}
      </Button>
    </div>
  );
}
