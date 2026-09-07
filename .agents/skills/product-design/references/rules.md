# Accepted rules

These rules record repeated decisions already owned by current product, design, and acceptance
sources. Their IDs remain stable when wording is clarified.

## rule/zed-owns-visual-primitives

**Status:** superseded by ADR-0002 and the approved GPUI Kit migration.
**Current rule:** use locked GPUI Kit components and active semantic tokens. Astesia owns
product composition and engine identity. Preserve the historical behavioral acceptance
contracts without reintroducing Zed dependencies.
**Source:** `docs/adr/0002-use-gpui-kit-for-the-desktop-shell.md`; `gpui-kit-design.md`.
**Bad:** add a Zed UI crate to obtain a familiar control.
**Good:** use the matching Kit control, or document a concrete product-specific gap.

## rule/capability-gates-actions

**Scope:** engine-specific navigation and actions.
**Rule:** expose an action only when the selected engine and current state support it. Unsupported
sections are absent rather than presented as empty-looking or enabled controls.
**Why:** a generic database UI otherwise promises destructive or editing behavior the driver cannot
honor.
**Exceptions:** none; adding capability requires Application Core and acceptance evidence.
**Source:** `docs/plans/gpui-milestone-0-acceptance.md`, capability matrix and D01-D11.
**Bad:** showing row editing for ClickHouse or MongoDB.
**Good:** preserve selection/copy/export while mutation entry points remain unavailable.

## rule/state-is-readable

**Scope:** Connection Profiles, tasks, errors, selection, and runtime status.
**Rule:** communicate state with text or an icon in addition to color.
**Why:** connection and failure meaning must survive color-vision differences and low-contrast
conditions.
**Exceptions:** color may decorate or identify an engine when no state meaning depends on it.
**Source:** `DESIGN.md`, The State-Is-Written Rule; milestone 0 C05.
**Bad:** a red dot as the only failure indication.
**Good:** a failure label or icon with the semantic color.

## rule/dirty-work-never-disappears-silently

**Scope:** query tabs, file open/restart, and staged grid edits.
**Rule:** block navigation or ask for an explicit discard choice before losing dirty text or staged
mutations. Cancellation preserves the work and active selection.
**Why:** operators must control when unsaved database work is abandoned.
**Exceptions:** none for reachable dirty state.
**Source:** milestone 0 S05, Q01, D06-D08, shortcut and confirmation contracts.
**Bad:** closing all tabs or sorting a grid and dropping pending work.
**Good:** name the unsaved scope and offer a concrete discard action plus Cancel.

## rule/destructive-names-target-and-consequence

**Scope:** profile deletion, object drop, Redis key deletion, restore, and destructive MCP work.
**Rule:** confirmation names the exact target and durable consequence before execution; cancellation
issues no mutation.
**Why:** generic confirmation hides scope and can cause irreversible data or credential loss.
**Exceptions:** a staged reversible change may use inline Undo/Discard until Save commits it.
**Source:** milestone 0 C03, D05, E04, O03, O08, and confirmation contract.
**Bad:** `Are you sure?` with an `OK` button.
**Good:** `Delete connection profile` plus the stored-credential consequence and a `Delete` action.

## rule/startup-preserves-native-state

**Scope:** repository and credential initialization.
**Rule:** run the Native State Probe before initialization; unreadable or corrupt state remains
untouched while the UI shows remediation.
**Why:** replacing uncertain state with an empty repository converts a recoverable startup failure
into data loss.
**Exceptions:** explicit user-authorized recovery outside the startup path.
**Source:** `PRODUCT.md`, `CONTEXT.md`, and milestone 0 S01.
**Bad:** initialize defaults after a repository parse failure.
**Good:** show the failure and a safe retry or remediation path.

## rule/terminal-outcome-is-truthful

**Scope:** background tasks, durable files, MCP lifecycle, and asynchronous profile operations.
**Rule:** distinguish Completed, Failed, Partial, Cancelled, and uncertain post-operation state. Emit
one terminal notification and preserve inspection or recovery context.
**Why:** dispatch completion or partial output is not proof of successful durable work.
**Exceptions:** none.
**Source:** milestone 0 O02-O07 and state/confirmation contracts.
**Bad:** report success after cancellation or when refresh fails after a mutation.
**Good:** say the mutation may have completed, lock stale UI, and require refresh before continuing.

## rule/editor-input-owns-editor-keys

**Scope:** query editor, inline fields, grid cell editors, command palette, and shortcuts.
**Rule:** global commands run only in their declared context and never consume normal editable or
IME composition input.
**Why:** keyboard speed is a primary workflow, but data entry and Chinese composition must remain
predictable.
**Exceptions:** an explicit focused overlay may own its navigation and confirmation keys.
**Source:** milestone 0 S03, Q06, and shortcut contract; milestone 1 IME acceptance.
**Bad:** `Mod+R` or Enter dispatching a workspace action from an active text-editing context.
**Good:** the active eligible view owns the shortcut and disabled handlers do not consume it.

## rule/engine-color-is-identity

**Scope:** engine marks, actions, and persistent surfaces.
**Rule:** use engine colors only to identify database type; actions and focus use semantic theme
roles.
**Why:** spending identity colors on action hierarchy makes both meanings ambiguous.
**Exceptions:** engine-specific visualizations may label a series with the same identity color when
the legend states the engine meaning.
**Source:** `DESIGN.md`, The Identity-Is-Not-Action Rule; `references/gpui-kit-design.md`.
**Bad:** make every MongoDB action green or Redis destructive button red because of engine identity.
**Good:** retain native action styling and use a small engine identity mark.
