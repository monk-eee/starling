# ADR-0007: Scope authentication to the MCP target

**Status:** Proposed

## Context and scope

[MCP sessions](../../src/mcp.rs) can use HTTP or HTTPS. HTTPS authenticates
the **server**; Starling does not yet authenticate the **client** to an MCP
target. The credentials for bird telemetry and for optional OTLP export are
for different peers and must not be reused.

## Proposal and alternatives

Keep anonymous targets simple. For a future authenticated target, let the
operator bind a credential reference to one exact HTTPS origin on the
machine making the request. Resolve the secret there, not in the controller
or a browser upload. Start with a bounded bearer-token use case; decide
separately whether OAuth acquisition/refresh or target mTLS is needed.
Do not send a credential over plaintext, follow redirects with it, or reuse
it for a different named endpoint. Missing or expired credentials must not
silently fall back to anonymous access.

Arbitrary headers or literal tokens in workload YAML would be easier to
implement but would turn ordinary scenarios, dashboard uploads and repro
commands into secret-bearing artifacts. A single global token would also
let an unrelated workload select its recipient.

## Consequences and verification

Until this is implemented, endpoints requiring client credentials cannot
be tested directly. Before shipping it, exercise two synthetic HTTPS
origins: only the intended one should receive the credential. Verify denied
credentials fail the run and that no secret occurs in diagnostics, live
frames, reports or [OTLP](0005-process-telemetry.md). The
[report privacy boundary](0004-keep-payloads-out-of-results.md) still applies.

## Decision history

Proposed 2026-11-23; acceptance not recorded.
