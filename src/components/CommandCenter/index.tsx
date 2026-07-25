import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Search } from 'lucide-react';
import { Input } from '@/components/ui/input';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';
import { cn } from '@/lib/utils';
import {
  canExecuteCommand,
  commandDefinitions,
  executeCommand,
  formatShortcut,
  useCommandHandler,
} from '@/lib/commands';
import {
  getMatchingCommands,
  hasOpenOverlay,
  isEditableTarget,
} from '@/lib/commands/keyboard';
import type { CommandDefinition, CommandId } from '@/lib/commands';

const setRootFontSize = (next: number | null) => {
  document.documentElement.style.fontSize = next === null ? '' : `${next}px`;
};

const adjustRootFontSize = (delta: number) => {
  const current = parseFloat(getComputedStyle(document.documentElement).fontSize);
  setRootFontSize(Math.max(10, Math.min(24, current + delta)));
};

export default function CommandCenter() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [selectedIndex, setSelectedIndex] = useState(0);

  useCommandHandler('app.commandPalette', () => setOpen(true));
  useCommandHandler('app.zoomIn', () => adjustRootFontSize(1));
  useCommandHandler('app.zoomOut', () => adjustRootFontSize(-1));
  useCommandHandler('app.zoomReset', () => setRootFontSize(null));

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (hasOpenOverlay()) return;

      const editable = isEditableTarget(event.target);
      const command = getMatchingCommands(event).find((candidate) => {
        if (event.repeat && !candidate.repeatable) return false;
        if (editable && !candidate.allowInEditable) return false;
        return canExecuteCommand(candidate.id);
      });
      if (!command) return;

      event.preventDefault();
      void executeCommand(command.id).catch((error) => {
        console.error(`Failed to execute command ${command.id}:`, error);
      });
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, []);

  const filteredCommands = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return commandDefinitions.filter((command) => {
      if (command.id === 'app.commandPalette') return false;
      if (!normalizedQuery) return true;
      const title = t(command.titleKey).toLocaleLowerCase();
      const category = t(command.categoryKey).toLocaleLowerCase();
      return title.includes(normalizedQuery)
        || category.includes(normalizedQuery)
        || command.id.toLocaleLowerCase().includes(normalizedQuery);
    });
  }, [query, t]);

  const runCommand = async (id: CommandId) => {
    if (!canExecuteCommand(id)) return;
    setOpen(false);
    setQuery('');
    await executeCommand(id);
  };

  const handleSearchKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setSelectedIndex((current) =>
        filteredCommands.length === 0 ? 0 : (current + 1) % filteredCommands.length
      );
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setSelectedIndex((current) =>
        filteredCommands.length === 0
          ? 0
          : (current - 1 + filteredCommands.length) % filteredCommands.length
      );
    } else if (event.key === 'Enter') {
      event.preventDefault();
      const command = filteredCommands[selectedIndex];
      if (command) void runCommand(command.id);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => {
      setOpen(nextOpen);
      setSelectedIndex(0);
      if (!nextOpen) setQuery('');
    }}>
      <DialogContent className="max-w-xl gap-0 overflow-hidden p-0">
        <DialogHeader className="sr-only">
          <DialogTitle>{t('commands.paletteTitle')}</DialogTitle>
          <DialogDescription>{t('commands.paletteDescription')}</DialogDescription>
        </DialogHeader>
        <div className="flex items-center gap-2 border-b px-3">
          <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Input
            autoFocus
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setSelectedIndex(0);
            }}
            onKeyDown={handleSearchKeyDown}
            placeholder={t('commands.searchPlaceholder')}
            className="h-12 border-0 px-0 shadow-none focus-visible:ring-0"
          />
        </div>
        <div className="max-h-96 overflow-y-auto p-2">
          {filteredCommands.length === 0 ? (
            <p className="px-3 py-8 text-center text-sm text-muted-foreground">
              {t('commands.noResults')}
            </p>
          ) : (
            filteredCommands.map((command: CommandDefinition, index) => {
              const enabled = canExecuteCommand(command.id);
              return (
                <button
                  key={command.id}
                  type="button"
                  disabled={!enabled}
                  className={cn(
                    'flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm',
                    enabled
                      ? 'text-foreground hover:bg-accent'
                      : 'cursor-not-allowed text-muted-foreground opacity-50',
                    index === selectedIndex && enabled && 'bg-accent'
                  )}
                  onMouseMove={() => setSelectedIndex(index)}
                  onClick={() => void runCommand(command.id)}
                >
                  <span className="min-w-0 flex-1 truncate">
                    <span className="mr-2 text-xs text-muted-foreground">
                      {t(command.categoryKey)}
                    </span>
                    {t(command.titleKey)}
                  </span>
                  {command.shortcuts[0] && (
                    <kbd className="shrink-0 rounded border bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground">
                      {formatShortcut(command.shortcuts[0])}
                    </kbd>
                  )}
                </button>
              );
            })
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
