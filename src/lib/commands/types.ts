export type CommandId =
  | 'app.commandPalette'
  | 'app.zoomIn'
  | 'app.zoomOut'
  | 'app.zoomReset'
  | 'workspace.newQuery'
  | 'workspace.toggleSidebar'
  | 'tabs.closeActive'
  | 'tabs.next'
  | 'tabs.previous'
  | 'query.execute'
  | 'query.executeCurrent'
  | 'query.openFile'
  | 'query.saveFile'
  | 'view.refresh'
  | 'view.save';

export interface CommandDefinition {
  id: CommandId;
  titleKey: string;
  categoryKey: string;
  shortcuts: string[];
  allowInEditable?: boolean;
  repeatable?: boolean;
}

export interface CommandHandler {
  run: () => void | Promise<void>;
  enabled: () => boolean;
  priority: number;
}
