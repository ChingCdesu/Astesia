# Resilience

Load when a change can load, fail, become stale, lose permission, mutate data, run asynchronously,
produce partial output, or be cancelled.

## Reachable-state map

For the affected workflow, include only states the system can enter:

- initial, empty, sparse, and populated;
- loading, refreshing, saving, running, and cancelling;
- validation, unsupported capability, disabled, permission, and Usage Lease blocked;
- stale revision or state changed elsewhere;
- recoverable error, terminal failure, partial completion, cancellation, and success.

Distinguish an empty result from failure, cancellation from platform error, and Partial from Failed.
Progress is monotonic for one task and a terminal notification appears once.

## Preservation

- Startup repository or credential failures remain visible and do not replace native data with an
  empty repository.
- Per-node catalog failures preserve previously loaded siblings and data.
- A failed query statement keeps earlier ordered results visible.
- Grid save failure rolls back the transaction and keeps staged changes editable.
- Stale revisions reject the mutation and refresh before further work.
- File-dialog cancellation preserves text and file identity.
- Background uncertainty says what may have changed and gives a concrete refresh or retry action.

## Consequential actions

Before commitment, name the exact object and scope, state the durable or credential consequence,
and say whether the action can be undone. Cancellation changes nothing. Refresh owning data only
after confirmed success. Do not use a success state for partial, cancelled, or unverified work.

Long-running output identifies its target Database Session, durable file or object scope, progress,
and terminal state. A retry must not silently duplicate durable work.
