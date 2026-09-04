# Product-design skill maintenance

## Load order

Read `SKILL.md`, then only the reference being changed and every canonical repository source it
cites. Read an exemplar when changing the decision it documents. Do not infer current paths from
historical acceptance documents.

## Governance

- Keep product terms in `CONTEXT.md`, product commitments in `PRODUCT.md`, Astesia-specific visual
  extensions in `DESIGN.md`, and runtime architecture in ADRs. The pinned Zed UI source owns base
  components, theme roles, density, typography, elevation, interaction, and accessibility. The
  skill routes to those owners instead of duplicating them.
- Reinspect the pinned Zed revision before changing `references/zed-design.md`; names and defaults
  from another Zed revision are not evidence for this repository.
- A new standard needs verified scope, rationale, evidence, exceptions, and human acceptance.
- Keep unverified candidates in `references/coverage-gaps.md`; do not phrase them as rules.
- Give accepted rules stable `rule/<id>` identifiers and keep each decision in one file.
- Use a linter only when source can identify the violation reliably with low false positives and a
  concrete remediation. Keep product judgment in prose.

## Validation

Run the skill validator, search the skill for unfinished placeholders, verify that every linked
reference exists, and run `git diff --check`. Changes to product behavior still require the normal
repository tests and native UI verification; documentation validation does not replace them.
