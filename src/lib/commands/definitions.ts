import type { CommandDefinition, CommandId } from './types';

export const commandDefinitions: CommandDefinition[] = [
  {
    id: 'app.commandPalette',
    titleKey: 'commands.commandPalette',
    categoryKey: 'commands.categories.app',
    shortcuts: ['Mod+Shift+P'],
    allowInEditable: true,
  },
  {
    id: 'workspace.newQuery',
    titleKey: 'commands.newQuery',
    categoryKey: 'commands.categories.workspace',
    shortcuts: ['Mod+N'],
    allowInEditable: true,
  },
  {
    id: 'workspace.toggleSidebar',
    titleKey: 'commands.toggleSidebar',
    categoryKey: 'commands.categories.workspace',
    shortcuts: ['Mod+B'],
    allowInEditable: true,
  },
  {
    id: 'tabs.closeActive',
    titleKey: 'commands.closeActiveTab',
    categoryKey: 'commands.categories.tabs',
    shortcuts: ['Mod+W'],
    allowInEditable: true,
  },
  {
    id: 'tabs.next',
    titleKey: 'commands.nextTab',
    categoryKey: 'commands.categories.tabs',
    shortcuts: ['Ctrl+Tab'],
    allowInEditable: true,
    repeatable: true,
  },
  {
    id: 'tabs.previous',
    titleKey: 'commands.previousTab',
    categoryKey: 'commands.categories.tabs',
    shortcuts: ['Ctrl+Shift+Tab'],
    allowInEditable: true,
    repeatable: true,
  },
  {
    id: 'query.execute',
    titleKey: 'commands.executeQuery',
    categoryKey: 'commands.categories.query',
    shortcuts: ['Mod+Enter'],
  },
  {
    id: 'query.executeCurrent',
    titleKey: 'commands.executeCurrentStatement',
    categoryKey: 'commands.categories.query',
    shortcuts: ['Mod+Shift+Enter'],
  },
  {
    id: 'query.openFile',
    titleKey: 'commands.openSqlFile',
    categoryKey: 'commands.categories.query',
    shortcuts: ['Mod+O'],
  },
  {
    id: 'query.saveFile',
    titleKey: 'commands.saveSqlFile',
    categoryKey: 'commands.categories.query',
    shortcuts: ['Mod+S'],
  },
  {
    id: 'view.refresh',
    titleKey: 'commands.refreshView',
    categoryKey: 'commands.categories.view',
    shortcuts: ['Mod+R'],
    allowInEditable: true,
  },
  {
    id: 'view.save',
    titleKey: 'commands.saveChanges',
    categoryKey: 'commands.categories.view',
    shortcuts: ['Mod+S'],
  },
  {
    id: 'app.zoomIn',
    titleKey: 'commands.zoomIn',
    categoryKey: 'commands.categories.app',
    shortcuts: ['Mod+=', 'Mod+Shift+='],
    allowInEditable: true,
    repeatable: true,
  },
  {
    id: 'app.zoomOut',
    titleKey: 'commands.zoomOut',
    categoryKey: 'commands.categories.app',
    shortcuts: ['Mod+-'],
    allowInEditable: true,
    repeatable: true,
  },
  {
    id: 'app.zoomReset',
    titleKey: 'commands.zoomReset',
    categoryKey: 'commands.categories.app',
    shortcuts: ['Mod+0'],
    allowInEditable: true,
  },
];

const commandById = new Map(commandDefinitions.map((command) => [command.id, command]));

export function getCommandDefinition(id: CommandId): CommandDefinition {
  const definition = commandById.get(id);
  if (!definition) {
    throw new Error(`Unknown command: ${id}`);
  }
  return definition;
}
