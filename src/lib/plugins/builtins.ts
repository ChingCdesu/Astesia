import { pluginRegistry } from './registry';

// Built-in JSON cell viewer
pluginRegistry.registerCellViewer({
  id: 'builtin-json',
  name: 'JSON Viewer',
  canHandle: (columnType) => {
    const t = columnType.toLowerCase();
    return t === 'json' || t === 'jsonb';
  },
  priority: 0,
  renderInline: (value) => {
    if (value === null) return null;
    const str = typeof value === 'string' ? value : JSON.stringify(value);
    return str.length > 50 ? str.slice(0, 50) + '...' : str;
  },
});

// Built-in UUID viewer (example of a simple type plugin)
pluginRegistry.registerCellViewer({
  id: 'builtin-uuid',
  name: 'UUID',
  canHandle: (columnType) => columnType.toLowerCase() === 'uuid',
  priority: 0,
  renderInline: (value) => value,
});

// Built-in array viewer (PostgreSQL arrays)
pluginRegistry.registerCellViewer({
  id: 'builtin-array',
  name: 'Array Viewer',
  canHandle: (columnType) => columnType.toLowerCase().startsWith('_') || columnType.toLowerCase() === 'array',
  priority: 0,
});
