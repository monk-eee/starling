# ADR-0001: Record architectural decisions alongside the code

**Status:** Proposed · **Proposed:** 2026-11-20 · **Acceptance:** Not recorded.

## Context

The [product spec](../SPEC.md) describes both features that work and features
we might build. For example, it describes a controller assigning work, but the
current RPC can only receive telemetry. A contributor needs to find that
distinction before changing the contract.

## Proposal

Keep one numbered record for each consequential choice about a public
interface, data boundary, or measurement. Put the alternatives, trade-off and
verification in the record. Keep command syntax in the README or developer
guide, and user-visible changes in the changelog.

Relying on commits alone preserves **what** changed but often not the rejected
alternative. Requiring a new ADR for every edit would bury the decisions among
routine fixes. This proposal does neither and adds no merge requirement.

## Verification and review

Starting at the [index](README.md), a contributor should be able to find a
relevant record, distinguish a proposal from an accepted decision, and follow
its evidence. The first three product records exercise that path. Adoption is
pending maintainer review; the presence of these files is not acceptance.
