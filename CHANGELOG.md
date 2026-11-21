# Changelog

User- and operator-visible changes to Starling. New changes go in
[changelog.d](changelog.d/README.md) until release preparation. The entries
below establish the unreleased baseline through `ecc6ff4`; the package version
in Cargo is not a release announcement.

The [pending fragments](changelog.d/) cover toolchain, telemetry, contributor
checks and coverage since that baseline. For the current checkout, use the
[install and smoke test](QUICKSTART.md#install-the-cli) and the
[logging guide](developer.md#process-telemetry).

## Unreleased

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

### Changed

- New workloads add no wait between requests by default. Existing positive
  waits are preserved; use `--wait 0` to override one.
- The flock distinguishes **reporting** from **running**. Missed heartbeats
  make a bird stale rather than counting a workload failure.

### Fixed

- On macOS and Linux, SIGTERM follows the graceful stop path so a container
  allowed to drain can write its partial report.
