import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Label } from '@/components/ui/label';
import { useConnectionStore } from '@/stores/connectionStore';
import { DbType } from '@/types/database';

interface CopyTableDialogProps {
  open: boolean;
  onClose: () => void;
  source: {
    connectionId: string;
    database: string;
    tableName: string;
    dbType: DbType;
  };
  target: {
    connectionId: string;
    database: string;
  };
}

export default function CopyTableDialog({ open, onClose, source, target }: CopyTableDialogProps) {
  const { t } = useTranslation();
  const { connections } = useConnectionStore();
  const [newTableName, setNewTableName] = useState(source.tableName);
  const [includeStructure, setIncludeStructure] = useState(true);
  const [includeData, setIncludeData] = useState(true);
  const [loading, setLoading] = useState(false);

  const sourceConn = connections.find((c) => c.id === source.connectionId);
  const targetConn = connections.find((c) => c.id === target.connectionId);

  const handleStartCopy = async () => {
    if (!newTableName.trim()) return;
    setLoading(true);
    try {
      await invoke('copy_table', {
        sourceConnectionId: source.connectionId,
        sourceDatabase: source.database,
        sourceTable: source.tableName,
        targetConnectionId: target.connectionId,
        targetDatabase: target.database,
        options: {
          include_structure: includeStructure,
          include_data: includeData,
          new_table_name: newTableName.trim(),
        },
      });
      onClose();
    } catch (e) {
      console.error('Copy table failed:', e);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t('tableCopy.title')}</DialogTitle>
          <DialogDescription>{t('tableCopy.sameTypeOnly')}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          {/* Source info */}
          <div className="rounded-md border p-3 space-y-1">
            <Label className="text-xs text-muted-foreground">{t('tableCopy.source')}</Label>
            <div className="text-sm">
              <span className="font-medium">{sourceConn?.name || source.connectionId}</span>
              <span className="mx-1.5 text-muted-foreground">/</span>
              <span>{source.database}</span>
              <span className="mx-1.5 text-muted-foreground">/</span>
              <span className="font-medium">{source.tableName}</span>
            </div>
          </div>

          {/* Target info */}
          <div className="rounded-md border p-3 space-y-1">
            <Label className="text-xs text-muted-foreground">{t('tableCopy.target')}</Label>
            <div className="text-sm">
              <span className="font-medium">{targetConn?.name || target.connectionId}</span>
              <span className="mx-1.5 text-muted-foreground">/</span>
              <span>{target.database}</span>
            </div>
          </div>

          {/* New table name */}
          <div>
            <Label className="mb-1.5 block">{t('tableCopy.newTableName')}</Label>
            <Input
              value={newTableName}
              onChange={(e) => setNewTableName(e.target.value)}
              placeholder={source.tableName}
            />
          </div>

          {/* Options */}
          <div className="grid grid-cols-2 gap-3">
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-gray-300"
                checked={includeStructure}
                onChange={(e) => setIncludeStructure(e.target.checked)}
              />
              {t('tableCopy.copyStructure')}
            </label>
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-gray-300"
                checked={includeData}
                onChange={(e) => setIncludeData(e.target.checked)}
              />
              {t('tableCopy.copyData')}
            </label>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            onClick={handleStartCopy}
            disabled={loading || !newTableName.trim() || (!includeStructure && !includeData)}
          >
            {t('tableCopy.start')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
