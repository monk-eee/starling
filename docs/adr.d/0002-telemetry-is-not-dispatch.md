# ADR-0002: Keep bird telemetry separate from workload execution

**Status:** Proposed · **Proposed:** 2026-11-20 · **Acceptance:** Not recorded.

## Context

The [gRPC contract](../../proto/flock.proto) defines `CheckIn(Heartbeat)` and
an empty `Receipt`. A [bird](../../src/bird.rs) chooses its workload locally;
the browser's **Start run** acts on the [controller's local run](../../src/dashboard.rs).
Several reporting birds need not be several running workloads or distinct hosts.

## Proposal and alternative

Keep check-ins observational. A new reporter must not receive a target or
start sending test requests because it joined the roster. A controller that
distributes a workload could coordinate a test across hosts, but it would
also need separate authorization, a global load budget and defined behavior
when a worker disappears. That is a different contract, not another field in
a heartbeat.

## Consequences and verification

An operator can see each bird's last measurements without uploading its
questions. The cost is that independent workers must be started locally; the
dashboard cannot replay its run on the flock. Revisit this record before
implementing [the distributed scheduling proposal](../SPEC.md#12-distributed-execution).

The [browser test](../../web/tests/flow.spec.mjs) observes an idle bird with
zero requests and checks its stale/rejoin transitions. The
[container check](../../tests/flock_containers.py) runs two real birds,
rejects an untrusted client and verifies that a restart does not add a third ID.
