import type { CommandHandler, CommandId } from './types';

const handlers = new Map<CommandId, Map<symbol, CommandHandler>>();

export function registerCommandHandler(
  id: CommandId,
  handler: CommandHandler
): () => void {
  const token = Symbol(id);
  const commandHandlers = handlers.get(id) ?? new Map<symbol, CommandHandler>();
  commandHandlers.set(token, handler);
  handlers.set(id, commandHandlers);

  return () => {
    const current = handlers.get(id);
    current?.delete(token);
    if (current?.size === 0) handlers.delete(id);
  };
}

const getEnabledHandler = (id: CommandId): CommandHandler | undefined =>
  Array.from(handlers.get(id)?.values() ?? [])
    .filter((handler) => handler.enabled())
    .sort((a, b) => b.priority - a.priority)[0];

export function canExecuteCommand(id: CommandId): boolean {
  return Boolean(getEnabledHandler(id));
}

export async function executeCommand(id: CommandId): Promise<boolean> {
  const handler = getEnabledHandler(id);
  if (!handler) return false;
  await handler.run();
  return true;
}
