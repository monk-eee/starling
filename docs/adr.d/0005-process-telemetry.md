# ADR-0005: Export process telemetry without exporting workloads

**Status:** Proposed

## Context and scope

An operator may want to see runs from several birds in an observability
backend without publishing their scenarios. The [flock heartbeat](../../proto/flock.proto)
serves live membership, not durable export. Existing [report privacy](0004-keep-payloads-out-of-results.md)
also applies to telemetry leaving a process.

## Proposal and alternatives

Keep local structured stderr logs as the default. Make an OTLP/HTTP exporter
an explicit build and runtime choice. Give each process its own providers,
record each scheduled request's elapsed time in a histogram, and correlate
run lifecycle logs with a single run span. Do not collect arbitrary tracing
fields from dependencies or serialize a workload, a report frame, or an error
chain into an exporter.

Export just the service and instance, role, public bird ID, fixed events, MCP
method, outcome, status, counts, timing and trace identifiers. Exclude target
origins and paths, arguments, responses, headers, session IDs, run names,
tool/prompt names and user-defined task labels. A per-call trace would make
individual requests easier to inspect but would add work to the measurement
path and increase the risk of disclosing input.

## Consequences and verification

The dashboard still works when no exporter exists. Aggregation requires the
collector to combine distributions; exported events are not proof of durable
storage. The [wire test](../../tests/cli/telemetry.rs) decodes all three signals
and checks them against the local report, including on workload failure.
Revisit the allowed fields before adding another attribute or sink.

## Decision history

Proposed 2026-11-21; acceptance not recorded.
