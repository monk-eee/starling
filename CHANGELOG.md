# Changelog

User- and operator-visible changes to Starling. New changes go in
[changelog.d](changelog.d/README.md) until release preparation. Published
versions are available from [GitHub Releases](https://github.com/monk-eee/starling/releases).

## Unreleased

## 0.2.0 - 2026-11-23

### Added

- [Performance acceptance gates](README.md#use-conformance-and-performance-as-deployment-gates)
  for `starling run`. Set any combination of `error_rate`, `p95`, `p99`, and
  `min_requests` in the workload's `assert` block. Starling evaluates the
  final scheduled requests after drain, excluding setup and discovery.
- The CLI exits nonzero if a configured gate fails or is incomplete. The
  summary and offline dashboard show each threshold, its observed value and
  result; JUnit includes the gate verdict. Conformance checks remain separate,
  and workloads without thresholds keep their previous behavior.

## 0.1.0 - 2026-11-21

First public source release. Follow the [quickstart](QUICKSTART.md#install-the-cli)
to build the embedded dashboard and install the CLI.

### Added

- An installable CLI with a self-contained MCP demo, YAML/TOML starter
  workloads, validation, headless execution and a live browser dashboard.
- Named tool and prompt requests, assertions, local endpoint selection and
  dashboard YAML import. Loading a file previews the workload without
  contacting the target.
- Standalone offline HTML, JSON, CSV and JUnit reports saved in a fresh
  directory for each run.
- A live flock view for independent birds. Select a bird to see its
  heartbeats, request counts and latency. Cross-host reporting uses mutual
  TLS; a heartbeat does not give the controller permission to assign work.
- [MCP protocol selection](README.md#test-an-mcp-protocol-revision) in the
  GUI, YAML/TOML or CLI. Choose `auto` or pin `2025-03-26`, `2025-06-18`,
  `2025-11-25` or `2026-07-28` over Streamable HTTP. A pinned run fails
  rather than silently accepting a different revision. Reports separate the
  requested revision from those negotiated by established sessions.
- A versioned one-shot conformance profile and a finite gRPC collector.
  Run `starling check --demo --all-protocols` or `starling collect --demo-birds 2`
  for per-rule evidence. The dashboard can check the selected or all revisions
  and download JSON without starting a load test. Missing worker evidence,
  unsupported revisions and inconclusive checks cannot produce a fleet pass.
- Structured lifecycle logs on stderr. An optional [OTLP/HTTP build](developer.md#process-telemetry)
  exports correlated run spans, logs, request counts and latency histograms
  to a collector selected by the operator.
- Separate [Rust and browser coverage reports](developer.md#measure-test-coverage)
  in CI, and [opt-in pre-commit checks](CONTRIBUTING.md#optional-pre-commit-hook)
  for formatting, staged changes and public files.

### Changed

- Building from source requires Rust 1.99 or newer; the checkout and Docker
  builder pin Rust 1.99.1.
- New workloads add no wait between requests by default. Existing positive
  waits are preserved; use `--wait 0` to override one.
- The flock distinguishes **reporting** from **running**. Missed heartbeats
  make a bird stale rather than counting a workload failure.

### Fixed

- On macOS and Linux, SIGTERM follows the graceful stop path so a container
  allowed to drain can write its partial report.
