# Architecture decision records

Use the [specification](../SPEC.md) for product direction, the
[developer guide](../../developer.md) for how the software works today, and
this directory for choices that a future change might otherwise undo.

| Record | Status | Read when changing |
|---|---|---|
| [0001: Record architectural decisions](0001-record-architecture-decisions.md) | Proposed | What deserves a record, and who can accept one |
| [0002: Separate bird telemetry from workload execution](0002-telemetry-is-not-dispatch.md) | Proposed | Bird registration, dashboard controls, target selection |
| [0003: Compute percentiles from distributions](0003-percentiles-need-distributions.md) | Proposed | Latency aggregation, fleet-wide charts |
| [0004: Keep workload payloads out of results](0004-keep-payloads-out-of-results.md) | Proposed | Report formats, live metrics, labels, diagnostics |

These are proposals, **not historical approvals**. Each product record links
to the implementation and a check for the current behavior. A passing test
does not accept an ADR.

To propose a decision, copy [template.md](template.md) to the next unused
four-digit number and a short slug, then add it to the table. Include the
alternative you would otherwise choose and a concrete way to tell if the
decision still holds. A maintainer accepting a record adds their name, the
date, and a public review reference, then updates this index. If the choice
changes later, write a new record and link both as superseding/superseded;
don't replace the old rationale. Routine fixes need no new ADR.
