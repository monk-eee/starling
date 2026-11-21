# ADR-0004: Keep workload inputs and raw responses out of shared results

**Status:** Proposed · **Proposed:** 2026-11-20 · **Acceptance:** Not recorded.

## Context

A run can contain private questions and return private text. Its standalone
HTML file can be copied without the workload file. The
[report writer](../../src/report.rs) serializes the run's public metrics frame,
not the original workload or MCP responses.

## Proposal and alternative

Keep counters, timings, outcomes, target **origin** and public labels in
results. Keep arguments, raw results, full target paths and session IDs out
of the live metrics stream and offline bundle. Don't add a full request trace
to make an assertion reproducible: a shareable HTML file has no way to revoke
that disclosure. Reproduce the assertion locally with a synthetic input.

## Consequences and verification

Reports are useful for comparing behavior but cannot reconstruct private
questions. Labels are visible by design: a sensitive run or task name is
**not** made safe by exporting it. Treat the full workload as private.

Run `cargo test --locked --test cli real_sessions_produce_consistent_redacted_offline_reports`
after [building the dashboard](../../developer.md#build-and-run-from-a-checkout).
That test checks the generated files for a sentinel payload. The
[browser flow](../../web/tests/flow.spec.mjs) also checks live frames and the
export. [ADR-0005](0005-process-telemetry.md) covers the separate, opt-in
run span and its allowed fields. It does not authorize full request traces.
