# ADR-0006: Report versioned, bounded MCP conformance evidence

**Status:** Proposed

## Context and scope

A successful load test exercises sampled operations. It cannot establish
which standard requirements passed, or whether every expected worker ran
them. Older [handshake-based revisions](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle)
and [per-request versioning](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
also require different evidence.

## Proposal and alternatives

Keep the deterministic [conformance profile](../../src/conformance.rs)
separate from load. Report each selected revision, observed negotiation,
stable rule ID, verdict and a bounded evidence code. Skip unadvertised
optional capability probes; record blocked and inconclusive work explicitly.
Do not infer full specification compliance from the SDK accepting a response.

Use a separate finite result-only RPC for distributed checking, rather than
overloading heartbeats or distributing commands. Expected worker IDs and
revision sets are fixed for the campaign. No remotely supplied workload,
target or executable enters the worker.

## Consequences and verification

New revisions join the shared registry. Changes in rule meaning require a
new suite version rather than silently changing the meaning of stored
evidence. The [wire tests](../../tests/cli/protocol.rs) include a catalog that
passes an older profile and fails a modern one. The [fleet tests](../../tests/cli/conformance.rs)
verify independent processes and a missing worker. See the [extension guide](../../developer.md#extending-the-conformance-suite)
for the scope of each rule.

Proposed 2026-11-21; acceptance not recorded.
