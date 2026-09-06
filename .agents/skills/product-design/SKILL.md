---
name: product-design
description: Shape, implement, review, copy-edit, or harden Astesia user-facing product changes. Use when work changes what users see, understand, choose, or do across GPUI workflows, interaction, accessibility, localization, and reachable states, including user-visible backend outcomes. Not for backend-only work with no shipped UI effect, tests-only changes, telemetry, packaging, or marketing.
---

# Astesia Product Design

Make Astesia correct for the operator, the product, and the system. Working code is insufficient:
choose the right behavior, communicate scope and consequence, cover reachable failure paths, and
verify the native surface.

## Operating Contract

- Start with the operator's job, product object, current state, and intended system change.
- Define the desired behavior, success signal, and non-goals before choosing a component or layout.
- Use product behavior and accepted repository decisions as evidence. Treat shipped code as evidence
  of what exists, not automatic proof that the pattern is correct.
- Separate verified facts, design decisions, assumptions, and open questions.
- Choose the smallest coherent intervention. Prefer a better default or direct behavior before
  adding configuration, surfaces, or abstractions.
- Resolve information architecture, semantics, interaction, and state behavior before visual craft.
- Design every reachable state that the affected workflow can enter.
- Verify visual or interaction claims in the running native application. Source inspection alone
  does not establish rendered quality.

This is the single user-facing design entry point for Astesia. Its interface-quality reference
contains the applicable craft and bounded-QA contract; do not route this desktop GPUI application
through browser, iOS, or Android design workflows.

## Request Modes

Resolve the narrowest mode supported by the user's verb and requested artifact.

| Mode | Typical request | Required behavior |
| --- | --- | --- |
| Shape | Design a flow or decide how a feature should work | Frame the job and evidence, compare material alternatives, then define flow, states, acceptance criteria, risks, and open decisions. Do not edit unless asked. |
| Implement | Build, fix, improve, or make compliant | Resolve material product choices, then implement the smallest coherent end-to-end change in scope. |
| Review | Audit, critique, inspect a screenshot, route, or diff | Inspect source and rendered evidence, then report prioritized findings. Do not edit unless asked. |
| Copy | Rewrite labels, errors, confirmation text, or accessible names | Change user-facing language and directly required UI code only. Report structural blockers without expanding into redesign. |
| Harden | Polish, production-ready, edge cases, accessibility, or resilience | Preserve the settled direction while fixing reachable state, recovery, responsive, accessibility, and finish defects. |

A URL, screenshot, file, or route identifies scope; it does not authorize edits by itself. A
material decision changes the job, default, scope, consequence, navigation, interaction surface,
or reachable states.

Mode precedence follows explicit authority: a no-edit or read-only constraint selects Review even
when the request says “production-ready”; Harden applies when the user asks to change the product.
Copy selects only when language is the requested outcome. Otherwise use the narrowest remaining
mode.

## Decision Authority

Resolve conflicts in this order:

1. The user's explicit goal and constraints.
2. Verified product behavior, user evidence, and system truth.
3. `PRODUCT.md`, `CONTEXT.md`, applicable ADRs, and milestone acceptance contracts.
4. The locked GPUI Kit component, theme, spacing, typography, interaction, and
   accessibility APIs for visual primitives.
5. `DESIGN.md` for Astesia-specific composition and semantic extensions, plus accepted rules in
   [references/rules.md](references/rules.md).
6. Verified adjacent patterns and documented exemplars in this repository.
7. General interface heuristics.

Historical acceptance documents own durable behavior but may name obsolete paths. Use the current
tree for mechanical paths and the acceptance contract for product intent.

## Workflow

### 1. Set scope and mode

Name the requested mode, affected [surface](references/surfaces.md), user-visible outcome, and
whether the request authorizes edits. Completion: the plan cannot silently absorb an adjacent
surface or a different mode.

### 2. Load product context

Read the applicable `AGENTS.md` chain and the source that owns the behavior. For material changes,
read `PRODUCT.md`, `CONTEXT.md`, and the relevant acceptance rows. For visual or interaction work,
also read [references/gpui-kit-design.md](references/gpui-kit-design.md), `DESIGN.md`, the incumbent GPUI
implementation, and the locked GPUI Kit source for every component being changed. Read the GPUI ADR when
changing the native runtime, editor integration, platform access, or Legacy Shell boundary.

Completion: every claimed product constraint has a current canonical source.

### 3. Model material decisions

For Shape, Implement, Harden, full Review, or any material workflow change, read
[references/product-judgment.md](references/product-judgment.md) and form its compact decision
brief before proposing UI.

Completion: every material choice is supported by evidence or marked as an open decision for the
user.

### 4. Map the surface and states

Read [references/surfaces.md](references/surfaces.md), then only the surface files it routes to.
Inventory entry points, visible regions, transitions, exits, return paths, and reachable states.

Completion: success, cancellation, recovery, stale data, and destructive consequences are covered
where the product can actually reach them.

### 5. Load focused guidance

| Need | Load |
| --- | --- |
| Product, flow, default, scope, or consequence | [product-judgment.md](references/product-judgment.md) |
| Visual implementation, material visual change, or full review | [gpui-kit-design.md](references/gpui-kit-design.md) + [interface-quality.md](references/interface-quality.md) |
| Labels, errors, confirmations, accessible names, or terminology | [copy.md](references/copy.md) |
| Loading, empty, stale, permission, partial, cancellation, or destructive behavior | [resilience.md](references/resilience.md) |
| Established cross-surface decisions | [rules.md](references/rules.md) |
| Missing or weak evidence | [coverage-gaps.md](references/coverage-gaps.md) |

### 6. Decide, then implement

For every non-mechanical change, be able to state the user problem, selected behavior, consequence,
supporting evidence, and why the intervention is the smallest coherent one. Keep business rules in
Application Core and capability ownership in the engine model; GPUI presents those decisions.

Implement only when the request authorizes it. Preserve unrelated behavior and do not invent usage,
security, performance, or support claims.

### 7. Verify

- Confirm the primary job and acceptance criteria.
- Run repository checks proportional to the change.
- Exercise every materially changed reachable state, including cancellation and recovery.
- Verify Simplified Chinese and English when copy, layout, or accessible labels change.
- Verify keyboard order, focus ownership, shortcuts, and IME behavior when interaction changes.
- Verify light, dark, compact, and wide layouts when the affected surface renders in them.
- Confirm that changed primitives use GPUI Kit components and semantic theme APIs, or document
  why no suitable Kit primitive exists.
- Manually exercise affected engine capabilities and unsupported-action absence when relevant.
- For visible changes, inspect the native application in one batched pass, fix the observed defects
  together, then perform at most one confirmation pass.

State which platforms, engines, states, and rendered surfaces were actually exercised. Do not
generalize a static check or one platform into runtime verification elsewhere.

Use one of these evidence levels:

- **Rendered-verified:** the affected native surface and stated interaction were exercised.
- **Source-verified:** source and contracts were inspected, but visual, focus, accessibility, or
  runtime behavior remains explicitly unverified. A read-only review may complete at this level.
- **Blocked:** the requested conclusion depends on runtime evidence that cannot be obtained safely.
  Report the missing environment or artifact instead of guessing.

Read-only work never creates critique snapshots, screenshots, fixtures, or other persisted review
artifacts unless the user separately authorizes them.

## Review Output

Lead with findings ordered by user impact:

- **P0:** blocks the primary job, creates severe accessibility failure, or risks unrecoverable harm.
- **P1:** likely task failure, misleading consequence, missing critical state, or major platform or
  accessibility defect.
- **P2:** meaningful friction, inconsistency, weak hierarchy, or recoverability problem.
- **P3:** minor craft or consistency improvement.

Each finding names the rendered location or file/line, verification status, canonical source, user
consequence, and smallest concrete fix. If no finding survives verification, say so.

Name the selected mode, loaded surfaces, loaded references, and evidence level so routing and
coverage can be reviewed independently from the findings.

## Skill Integrity

Add or change a standard only after verifying current sources and receiving human acceptance. Give
rules stable IDs and record scope, rationale, evidence, exceptions, and a bad/good example. Put
deterministic, low-false-positive checks in code; keep contextual judgment in prose. One screenshot,
file, or reviewer comment remains evidence, not a universal rule.
