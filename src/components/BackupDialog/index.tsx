import { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Label } from '@/components/ui/label';
import { TableInfo } from '@/types/database';

interface BackupDialogProps {
  open: boolean;
  onClose: () => void;
  connectionId: string;
  database: string;
}

export default function BackupDialog({ open, onClose, connectionId, database }: BackupDialogProps) {
  const { t } = useTranslation();
  const [tables, setTables] = useState<TableInfo[]>([]);
  const [selectedTables, setSelectedTables] = useState<Set<string>>(new Set());
  const [includeStructure, setIncludeStructure] = useState(true);
  const [includeData, setIncludeData] = useState(true);
  const [addDropTable, setAddDropTable] = useState(false);
  const [addDropIfExists, setAddDropIfExists] = useState(false);
  const [outputPath, setOutputPath] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (open && connectionId && database) {
      invoke<TableInfo[]>('get_tables', { connectionId, database }).then((result) => {
        setTables(result);
        setSelectedTables(new Set(result.map((t) => t.name)));
      });
    }
  }, [open, connectionId, database]);

  const handleSelectAll = () => {
    setSelectedTables(new Set(tables.map((t) => t.name)));
  };

  const handleDeselectAll = () => {
    setSelectedTables(new Set());
  };

  const toggleTable = (name: string) => {
    setSelectedTables((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const handleBrowse = async () => {
    const filePath = await save({
      defaultPath: `${database}_backup.sql`,
      filters: [{ name: 'SQL Files', extensions: ['sql'] }],
    });
    if (filePath) {
      setOutputPath(filePath);
    }
  };

  const handleStartBackup = async () => {
    if (!outputPath) return;
    setLoading(true);
    try {
      await invoke('start_backup', {
        connectionId,
        database,
        options: {
          tables: Array.from(selectedTables),
          include_structure: includeStructure,
          include_data: includeData,
          add_drop_table: addDropTable,
          add_drop_if_exists: addDropIfExists,
          output_path: outputPath,
        },
      });
      onClose();
    } catch (e) {
      console.error('Backup failed:', e);
    } finally {
      setLoading(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) onClose(); }}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>{t('backup.title')}</DialogTitle>
          <DialogDescription>{database}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-4">
          {/* Table selection */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <Label>{t('backup.selectTables')}</Label>
              <div className="flex gap-2">
                <Button variant="ghost" size="sm" onClick={handleSelectAll}>
                  {t('backup.selectAll')}
                </Button>
                <Button variant="ghost" size="sm" onClick={handleDeselectAll}>
                  {t('backup.deselectAll')}
                </Button>
              </div>
            </div>
            <div className="max-h-40 overflow-y-auto rounded-md border p-2 space-y-1">
              {tables.map((table) => (
                <label
                  key={table.name}
                  className="flex items-center gap-2 cursor-pointer rounded px-1.5 py-1 text-sm hover:bg-accent"
                >
                  <input
                    type="checkbox"
                    className="h-3.5 w-3.5 rounded border-gray-300"
                    checked={selectedTables.has(table.name)}
                    onChange={() => toggleTable(table.name)}
                  />
                  {table.name}
                </label>
              ))}
            </div>
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
              {t('backup.structure')}
            </label>
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-gray-300"
                checked={includeData}
                onChange={(e) => setIncludeData(e.target.checked)}
              />
              {t('backup.data')}
            </label>
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-gray-300"
                checked={addDropTable}
                onChange={(e) => setAddDropTable(e.target.checked)}
              />
              {t('backup.dropTable')}
            </label>
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                className="h-3.5 w-3.5 rounded border-gray-300"
                checked={addDropIfExists}
                onChange={(e) => setAddDropIfExists(e.target.checked)}
              />
              {t('backup.dropIfExists')}
            </label>
          </div>

          {/* Output path */}
          <div>
            <Label className="mb-1.5 block">{t('backup.outputPath')}</Label>
            <div className="flex gap-2">
              <input
                type="text"
                readOnly
                value={outputPath}
                placeholder={t('backup.outputPath')}
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
            onClick={handleStartBackup}
            disabled={loading || !outputPath || selectedTables.size === 0}
          >
            {t('backup.start')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
