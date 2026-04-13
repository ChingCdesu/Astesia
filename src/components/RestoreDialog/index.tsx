import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { notify } from '@/stores/notificationStore';

interface RestoreDialogProps {
  open: boolean;
  onClose: () => void;
  connectionId: string;
  database: string;
}

export default function RestoreDialog({ open: isOpen, onClose, connectionId, database }: RestoreDialogProps) {
  const { t } = useTranslation();
  const [filePath, setFilePath] = useState('');
  const handleBrowse = async () => {
    const selected = await open({
      filters: [{ name: 'SQL Files', extensions: ['sql'] }],
      multiple: false,
    });
    if (selected) {
      setFilePath(selected as string);
    }
  };

  const handleStartRestore = async () => {
    if (!filePath) return;
    try {
      await invoke('start_restore', {
        connectionId,
        database,
        filePath,
      });
      notify.success(t('backup.restore'), t('backup.success'));
      onClose();
    } catch (e) {
      console.error('Restore failed:', e);
      notify.error(t('backup.restore'), String(e));
    }
  };

  return (
    <Dialog open={isOpen} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle>{t('backup.restore')}</DialogTitle>
          <DialogDescription>{database}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          <div>
            <Label className="mb-1.5 block">{t('backup.restoreFile')}</Label>
            <div className="flex gap-2">
              <input
                type="text"
                readOnly
                value={filePath}
                placeholder={t('backup.restoreFile')}
                className="flex-1 rounded-md border bg-background px-3 py-1.5 text-sm"
              />
              <Button variant="outline" size="sm" onClick={handleBrowse}>
                {t('backup.browse')}
              </Button>
            </div>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            onClick={handleStartRestore}
            disabled={!filePath}
          >
            {t('backup.startRestore')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
