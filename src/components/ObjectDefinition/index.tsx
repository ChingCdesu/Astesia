import { useState, useEffect } from 'react';
import Editor from '@monaco-editor/react';
import { invoke } from '@tauri-apps/api/core';
import { useThemeStore } from '@/stores/themeStore';
import { ViewInfo, FunctionInfo, ProcedureInfo } from '@/types/database';
import { Loader2, FileCode } from 'lucide-react';

interface Props {
  connectionId: string;
  database: string;
  objectName: string;
  objectType: 'view' | 'function' | 'procedure';
}

export default function ObjectDefinition({ connectionId, database, objectName, objectType }: Props) {
  const [definition, setDefinition] = useState<string>('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const resolvedTheme = useThemeStore((s) => s.resolvedTheme);

  useEffect(() => {
    const fetchDefinition = async () => {
      setLoading(true);
      setError(null);
      try {
        let def: string | undefined;
        if (objectType === 'view') {
          const views = await invoke<ViewInfo[]>('get_views', { connectionId, database });
          const found = views.find((v) => v.name === objectName);
          def = found?.definition;
        } else if (objectType === 'function') {
          const functions = await invoke<FunctionInfo[]>('get_functions', { connectionId, database });
          const found = functions.find((f) => f.name === objectName);
          def = found?.definition;
        } else if (objectType === 'procedure') {
          const procedures = await invoke<ProcedureInfo[]>('get_procedures', { connectionId, database });
          const found = procedures.find((p) => p.name === objectName);
          def = found?.definition;
        }
        setDefinition(def || `-- 无法获取 ${objectName} 的定义`);
      } catch (e) {
        setError(String(e));
      } finally {
        setLoading(false);
      }
    };
    fetchDefinition();
  }, [connectionId, database, objectName, objectType]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-muted-foreground">
        <Loader2 className="h-5 w-5 animate-spin" />
        <span>加载中...</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-destructive">
        <span>加载失败: {error}</span>
      </div>
    );
  }

  const typeLabels: Record<string, string> = {
    view: '视图',
    function: '函数',
    procedure: '存储过程',
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-10 shrink-0 items-center gap-2 border-b bg-muted/30 px-4">
        <FileCode className="h-4 w-4 text-muted-foreground" />
        <span className="text-sm font-medium">{typeLabels[objectType]}: {objectName}</span>
      </div>
      <div className="flex-1">
        <Editor
          height="100%"
          language="sql"
          theme={resolvedTheme === 'dark' ? 'vs-dark' : 'vs'}
          value={definition}
          options={{
            readOnly: true,
            minimap: { enabled: false },
            fontSize: 14,
            lineNumbers: 'on',
            scrollBeyondLastLine: false,
            wordWrap: 'on',
            automaticLayout: true,
          }}
        />
      </div>
    </div>
  );
}
