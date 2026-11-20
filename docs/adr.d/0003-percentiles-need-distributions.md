# ADR-0003: Compute latency percentiles from distributions, not percentiles

**Status:** Proposed · **Proposed:** 2026-11-20 · **Acceptance:** Not recorded.

## Context

A slow operation sampled once and a fast operation sampled a thousand times
must not contribute equally to a run's p95. Even a count-weighted average of
their p95s does not recover the combined distribution. The
[local metrics collector](../../src/metrics.rs) retains HDR histograms; the
[bird heartbeat](../../proto/flock.proto) carries a p95, not a histogram.

## Proposal and alternative

Merge the relevant histograms before computing a run percentile. Exclude
session setup and discovery from workload KPIs. Show a bird's p95 as its own;
do not derive a flock-wide p95 from the values displayed on bird tiles.
Sending mergeable distributions could support a fleet percentile later, at
the cost of a new wire contract and rules for missing or duplicate samples.

## Consequences and verification

There is deliberately no fleet-wide latency number today. For the local
regression, run `cargo test --locked merge_samples_not_percentiles_and_keep_final_partial_bucket`
after [building the dashboard](../../developer.md#build-and-run-from-a-checkout).
The case combines unequal sample populations and checks the final partial
interval. It is not evidence that cross-worker aggregation exists.
