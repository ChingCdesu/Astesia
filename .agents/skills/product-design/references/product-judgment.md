# Product judgment

Load for Shape, Implement, Harden, full Review, or any change to a workflow, default, scope,
consequence, navigation, interaction surface, or reachable states.

## Decision brief

Write a compact internal brief before proposing UI:

- **User and job:** who is acting, what they need to accomplish, and in what operating context.
- **Current behavior:** the verified entry point, state, limitation, and system behavior.
- **Desired outcome:** the observable behavior and success signal.
- **Non-goals:** adjacent surfaces, engines, migrations, or policies outside the request.
- **Product object:** the exact Connection Profile, Database Session, query, result, catalog object,
  grid mutation, task, or MCP state being acted on.
- **Action and consequence:** what changes, where it persists, who or what else is affected, and
  whether it is reversible.
- **Authority:** engine capability, permission, Usage Lease, revision, validation, or platform
  boundary that enables or blocks the action.
- **States:** every reachable start, progress, success, empty, cancellation, stale, partial, and
  failure outcome relevant to the decision.
- **Evidence:** canonical document, verified system behavior, accepted rule, or adjacent pattern.
- **Assumptions and open decisions:** facts not yet verified and choices only the user can settle.

## Decision tests

- **Object:** Can the operator tell exactly what will change?
- **Scope:** Can they tell which profile, session, engine, database, object, rows, file, or task is
  included?
- **Consequence:** Does the interface explain destructive, durable, credential, or partial-output
  effects before commitment?
- **Default:** Does the common safe path work without introducing another setting?
- **Capability:** Is the action present only where the engine and current state support it?
- **Continuity:** Are active context, input, staged edits, and prior results preserved unless the
  operator explicitly discards them?
- **Recovery:** After failure or cancellation, does the interface show the truthful state and a safe
  next action?

When alternatives materially differ, compare the job fit, added concepts, failure surface,
reversibility, and consistency with canonical behavior. Completion requires one selected direction
or an explicit open decision; a blended compromise is not a decision.
