import { commandDefinitions } from './definitions';
import type { CommandDefinition, CommandId } from './types';

const isMacPlatform = (): boolean =>
  typeof navigator !== 'undefined' && /Mac|iPhone|iPad|iPod/i.test(navigator.userAgent);

const normalizeEventKey = (event: KeyboardEvent): string => {
  if (event.key === '+') return '=';
  return event.key.length === 1 ? event.key.toLowerCase() : event.key.toLowerCase();
};

const matchesShortcut = (event: KeyboardEvent, shortcut: string): boolean => {
  const parts = shortcut.split('+');
  const expectedKey = parts.at(-1)?.toLowerCase();
  if (!expectedKey) return false;

  const expectsMod = parts.includes('Mod');
  const expectsCtrl = parts.includes('Ctrl');
  const expectsShift = parts.includes('Shift');
  const expectsAlt = parts.includes('Alt');
  const mac = isMacPlatform();

  const expectedMeta = expectsMod && mac;
  const expectedCtrl = expectsCtrl || (expectsMod && !mac);

  return (
    event.metaKey === expectedMeta
    && event.ctrlKey === expectedCtrl
    && event.shiftKey === expectsShift
    && event.altKey === expectsAlt
    && normalizeEventKey(event) === expectedKey
  );
};

export function getMatchingCommands(event: KeyboardEvent): CommandDefinition[] {
  return commandDefinitions.filter((command) =>
    command.shortcuts.some((shortcut) => matchesShortcut(event, shortcut))
  );
}

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return Boolean(
    target.closest(
      'input, textarea, select, [contenteditable="true"], [contenteditable=""], .monaco-editor'
    )
  );
}

export function hasOpenOverlay(): boolean {
  return Boolean(
    document.querySelector(
      '[role="dialog"][data-state="open"], [role="menu"][data-state="open"], '
      + '[data-radix-popper-content-wrapper] [data-state="open"]'
    )
  );
}

const formatKeyForMac = (key: string): string => {
  const aliases: Record<string, string> = {
    Enter: '↵',
    Tab: '⇥',
  };
  return aliases[key] ?? key;
};

export function formatShortcut(shortcut: string): string {
  const parts = shortcut.split('+');
  if (!isMacPlatform()) return parts.map((part) => part === 'Mod' ? 'Ctrl' : part).join('+');

  const key = parts.at(-1) ?? '';
  const modifiers = parts.slice(0, -1).map((part) => {
    if (part === 'Mod') return '⌘';
    if (part === 'Ctrl') return '⌃';
    if (part === 'Shift') return '⇧';
    if (part === 'Alt') return '⌥';
    return part;
  });
  return `${modifiers.join('')}${formatKeyForMac(key)}`;
}

export function getPrimaryShortcut(id: CommandId): string {
  const command = commandDefinitions.find((definition) => definition.id === id);
  return command?.shortcuts[0] ? formatShortcut(command.shortcuts[0]) : '';
}
