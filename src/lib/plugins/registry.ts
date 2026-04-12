import type { CellValueViewerPlugin, DataViewerPlugin, SidebarPlugin } from './types';

class PluginRegistry {
  private cellViewers: CellValueViewerPlugin[] = [];
  private dataViewers: DataViewerPlugin[] = [];
  private sidebarPlugins: SidebarPlugin[] = [];

  registerCellViewer(plugin: CellValueViewerPlugin) {
    this.cellViewers.push(plugin);
    this.cellViewers.sort((a, b) => b.priority - a.priority);
  }

  registerDataViewer(plugin: DataViewerPlugin) {
    this.dataViewers.push(plugin);
  }

  registerSidebarPlugin(plugin: SidebarPlugin) {
    this.sidebarPlugins.push(plugin);
  }

  getCellViewerForType(columnType: string, dbType: string): CellValueViewerPlugin | undefined {
    return this.cellViewers.find(p => p.canHandle(columnType, dbType as any));
  }

  getAllCellViewers(): CellValueViewerPlugin[] {
    return [...this.cellViewers];
  }

  getDataViewers(dbType?: string): DataViewerPlugin[] {
    if (!dbType) return [...this.dataViewers];
    return this.dataViewers.filter(p => p.supportedDbTypes === 'all' || p.supportedDbTypes.includes(dbType as any));
  }

  getSidebarPlugins(dbType?: string): SidebarPlugin[] {
    if (!dbType) return [...this.sidebarPlugins];
    return this.sidebarPlugins.filter(p => p.supportedDbTypes === 'all' || p.supportedDbTypes.includes(dbType as any));
  }
}

export const pluginRegistry = new PluginRegistry();
