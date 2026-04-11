import { useState, useEffect } from 'react';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Badge } from '@/components/ui/badge';
import { ScrollArea } from '@/components/ui/scroll-area';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { ColumnInfo, IndexInfo } from '@/types/database';
import { Loader2 } from 'lucide-react';

interface Props {
  connectionId: string;
  database: string;
  table: string;
}

export default function TableStructure({ connectionId, database, table }: Props) {
  const { t } = useTranslation();
  const [columns, setColumns] = useState<ColumnInfo[]>([]);
  const [indexes, setIndexes] = useState<IndexInfo[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    const load = async () => {
      setLoading(true);
      try {
        const [cols, idxs] = await Promise.all([
          invoke<ColumnInfo[]>('get_columns', { connectionId, database, table }),
          invoke<IndexInfo[]>('get_indexes', { connectionId, database, table }),
        ]);
        setColumns(cols);
        setIndexes(idxs);
      } catch (e) {
        console.error('Failed to load structure:', e);
      } finally {
        setLoading(false);
      }
    };
    load();
  }, [connectionId, database, table]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
        <span>{t('common.loading')}</span>
      </div>
    );
  }

  return (
    <Tabs defaultValue="columns" className="flex h-full flex-col p-4">
      <TabsList className="mb-3 self-start">
        <TabsTrigger value="columns">{t('table.structure')} ({columns.length})</TabsTrigger>
        <TabsTrigger value="indexes">{t('table.indexes')} ({indexes.length})</TabsTrigger>
      </TabsList>

      <TabsContent value="columns" className="flex-1 overflow-hidden">
        <ScrollArea className="h-full">
          <table className="w-full text-sm">
            <thead className="sticky top-0 z-10">
              <tr className="border-b bg-muted/60">
                <th className="w-12 px-3 py-2.5 text-center text-xs font-medium text-muted-foreground">#</th>
                <th className="px-4 py-2.5 text-left text-xs font-medium">{t('table.column')}</th>
                <th className="px-4 py-2.5 text-left text-xs font-medium">{t('table.type')}</th>
                <th className="w-20 px-4 py-2.5 text-center text-xs font-medium">{t('table.nullable')}</th>
                <th className="w-20 px-4 py-2.5 text-center text-xs font-medium">{t('table.primaryKey')}</th>
                <th className="px-4 py-2.5 text-left text-xs font-medium">{t('table.defaultValue')}</th>
                <th className="px-4 py-2.5 text-left text-xs font-medium">{t('table.comment')}</th>
              </tr>
            </thead>
            <tbody>
              {columns.map((col, i) => (
                <tr key={i} className="border-b transition-colors hover:bg-muted/30">
                  <td className="px-3 py-2 text-center text-xs text-muted-foreground">{i + 1}</td>
                  <td className="px-4 py-2 font-medium">{col.name}</td>
                  <td className="px-4 py-2">
                    <Badge variant="info" className="font-mono text-[11px]">{col.data_type}</Badge>
                  </td>
                  <td className="px-4 py-2 text-center">
                    <Badge variant={col.nullable ? 'success' : 'destructive'} className="text-[10px]">
                      {col.nullable ? 'YES' : 'NO'}
                    </Badge>
                  </td>
                  <td className="px-4 py-2 text-center">
                    {col.is_primary_key && <Badge variant="warning" className="text-[10px]">PK</Badge>}
                  </td>
                  <td className="px-4 py-2 font-mono text-xs text-muted-foreground">
                    {col.default_value || '-'}
                  </td>
                  <td className="px-4 py-2 text-xs text-muted-foreground">
                    {col.comment || '-'}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </ScrollArea>
      </TabsContent>

      <TabsContent value="indexes" className="flex-1 overflow-hidden">
        <ScrollArea className="h-full">
          <table className="w-full text-sm">
            <thead className="sticky top-0 z-10">
              <tr className="border-b bg-muted/60">
                <th className="w-12 px-3 py-2.5 text-center text-xs font-medium text-muted-foreground">#</th>
                <th className="px-4 py-2.5 text-left text-xs font-medium">{t('table.indexName')}</th>
                <th className="px-4 py-2.5 text-left text-xs font-medium">{t('table.indexColumns')}</th>
                <th className="w-20 px-4 py-2.5 text-center text-xs font-medium">{t('table.unique')}</th>
                <th className="w-24 px-4 py-2.5 text-center text-xs font-medium">{t('table.primaryKey')}</th>
              </tr>
            </thead>
            <tbody>
              {indexes.map((idx, i) => (
                <tr key={i} className="border-b transition-colors hover:bg-muted/30">
                  <td className="px-3 py-2 text-center text-xs text-muted-foreground">{i + 1}</td>
                  <td className="px-4 py-2 font-medium">{idx.name}</td>
                  <td className="px-4 py-2 font-mono text-xs">{idx.columns.join(', ')}</td>
                  <td className="px-4 py-2 text-center">
                    {idx.is_unique && <Badge variant="info" className="text-[10px]">UNI</Badge>}
                  </td>
                  <td className="px-4 py-2 text-center">
                    {idx.is_primary && <Badge variant="warning" className="text-[10px]">PRI</Badge>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </ScrollArea>
      </TabsContent>
    </Tabs>
  );
}
