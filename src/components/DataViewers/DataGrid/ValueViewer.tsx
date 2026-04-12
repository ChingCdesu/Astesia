import { useState, useEffect } from 'react';
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter,
} from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Copy, Braces, Save } from 'lucide-react';
import Editor from '@monaco-editor/react';
import { useThemeStore } from '@/stores/themeStore';

interface ValueViewerProps {
  open: boolean;
  onClose: () => void;
  value: any;
  columnName: string;
  columnType: string;
  readOnly?: boolean;
  onSave?: (newValue: any) => void;
}

function isJsonType(type: string): boolean {
  const dt = type.toLowerCase();
  return dt === 'json' || dt === 'jsonb';
}

function isTextType(type: string): boolean {
  const dt = type.toLowerCase();
  return dt.includes('varchar') || dt.includes('text') || dt.includes('char') || dt.includes('nvarchar') || dt.includes('ntext') || dt.includes('longtext') || dt.includes('mediumtext');
}

function formatValue(value: any, type: string): string {
  if (value === null || value === undefined) return '';
  if (isJsonType(type) || typeof value === 'object') {
    try {
      return JSON.stringify(typeof value === 'string' ? JSON.parse(value) : value, null, 2);
    } catch {
      return typeof value === 'object' ? JSON.stringify(value) : String(value);
    }
  }
  return String(value);
}

export default function ValueViewer({ open, onClose, value, columnName, columnType, readOnly, onSave }: ValueViewerProps) {
  const resolvedTheme = useThemeStore((s) => s.resolvedTheme);
  const isJson = isJsonType(columnType) || typeof value === 'object';
  const isLargeText = isTextType(columnType) || (typeof value === 'string' && value.length > 100);
  const useMonaco = isJson || isLargeText;
  const language = isJson ? 'json' : 'plaintext';

  const [editorValue, setEditorValue] = useState('');

  useEffect(() => {
    setEditorValue(formatValue(value, columnType));
  }, [value, columnType]);

  const handleFormat = () => {
    try {
      const parsed = JSON.parse(editorValue);
      setEditorValue(JSON.stringify(parsed, null, 2));
    } catch {
      // not valid JSON, do nothing
    }
  };

  const handleCopy = () => {
    navigator.clipboard.writeText(editorValue);
  };

  const handleSave = () => {
    if (!onSave) return;
    if (isJson) {
      try {
        const parsed = JSON.parse(editorValue);
        onSave(parsed);
      } catch {
        // Save as string if not valid JSON
        onSave(editorValue);
      }
    } else {
      onSave(editorValue);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => { if (!o) onClose(); }}>
      <DialogContent className="max-w-2xl max-h-[80vh] flex flex-col">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-sm font-medium">
            <span>{columnName}</span>
            <span className="text-xs font-normal text-muted-foreground">({columnType})</span>
          </DialogTitle>
          <DialogDescription className="sr-only">
            查看和编辑 {columnName} 的值
          </DialogDescription>
        </DialogHeader>

        <div className="flex-1 min-h-0 overflow-hidden rounded border">
          {useMonaco ? (
            <Editor
              height="400px"
              language={language}
              theme={resolvedTheme === 'dark' ? 'vs-dark' : 'vs'}
              value={editorValue}
              onChange={(v) => setEditorValue(v ?? '')}
              options={{
                readOnly: !!readOnly,
                minimap: { enabled: false },
                scrollBeyondLastLine: false,
                fontSize: 13,
                lineNumbers: 'on',
                wordWrap: 'on',
                automaticLayout: true,
              }}
            />
          ) : (
            <pre className="h-[400px] overflow-auto p-4 text-xs font-mono whitespace-pre-wrap break-all text-foreground">
              {editorValue || '(empty)'}
            </pre>
          )}
        </div>

        <DialogFooter className="flex-row justify-between sm:justify-between">
          <div className="flex gap-2">
            {isJson && (
              <Button variant="outline" size="sm" onClick={handleFormat}>
                <Braces className="mr-1.5 h-3.5 w-3.5" />
                格式化
              </Button>
            )}
            <Button variant="outline" size="sm" onClick={handleCopy}>
              <Copy className="mr-1.5 h-3.5 w-3.5" />
              复制
            </Button>
          </div>
          <div className="flex gap-2">
            {!readOnly && onSave && (
              <Button size="sm" onClick={handleSave}>
                <Save className="mr-1.5 h-3.5 w-3.5" />
                保存
              </Button>
            )}
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
