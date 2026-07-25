import { useEffect } from 'react';
import { registerCommandHandler } from './registry';
import type { CommandId } from './types';

export function useCommandHandler(
  id: CommandId,
  run: () => void | Promise<void>,
  enabled = true,
  priority = 0
): void {
  useEffect(
    () => registerCommandHandler(id, {
      run,
      enabled: () => enabled,
      priority,
    }),
    [enabled, id, priority, run]
  );
}
