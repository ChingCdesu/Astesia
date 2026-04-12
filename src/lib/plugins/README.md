# Astesia Plugin System

## Cell Value Viewer Plugins

Register a custom viewer for specific column types:

```typescript
import { pluginRegistry } from '@/lib/plugins';

pluginRegistry.registerCellViewer({
  id: 'my-geojson-viewer',
  name: 'GeoJSON Map',
  canHandle: (columnType, dbType) => 
    columnType.toLowerCase().includes('geometry') || 
    columnType.toLowerCase().includes('geography'),
  priority: 10, // Higher than built-in (0)
  renderViewer: ({ value, columnName }) => {
    // Return a React element that renders a map
    return <MyMapComponent data={value} />;
  },
});
```

## Data Viewer Plugins

Register an entire tab view:

```typescript
pluginRegistry.registerDataViewer({
  id: 'my-graph-viewer',
  name: 'Graph View',
  supportedDbTypes: ['postgresql'],
  render: ({ connectionId, database, table }) => {
    return <MyGraphViewer ... />;
  },
});
```

## Sidebar Plugins

Add custom context menu items:

```typescript
pluginRegistry.registerSidebarPlugin({
  id: 'my-analytics',
  name: 'Analytics',
  supportedDbTypes: 'all',
  databaseMenuItems: ({ connectionId, database }) => (
    <ContextMenuItem onClick={() => ...}>
      Run Analytics
    </ContextMenuItem>
  ),
});
```
