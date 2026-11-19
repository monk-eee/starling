# Starling

Starling runs repeatable workloads against a **local MCP server** and shows what
happened to each tool call. This is the first vertical slice in
[the full technical specification, section 18](docs/SPEC.md#18-recommended-first-implementation-slice),
not the entire v0.1 roadmap.

## Try it

Building from source requires rustup, a native linker (MSVC C++ build tools and
a Windows SDK on Windows), and Node.js 22.12+ with npm. Build the dashboard
before Cargo; the executable embeds it and needs neither Node nor a CDN at runtime.

```sh
npm --prefix web ci
npm --prefix web run build
cargo build --locked
cargo run --locked -- fixture
```

In a second terminal:

```sh
cargo run --locked -- run --config examples/local.toml --web
```

Open the printed dashboard URL, adjust **Load settings**, then click **Start run**.
The controls start with the workload file and any CLI overrides; changes apply to this run without
editing the file. **Stop run** closes
admission to new operations, drains outstanding calls up to their deadlines,
closes the sessions, and writes the report. The dashboard stays up for inspection
until Ctrl+C. Use **Configure next run** to tune and repeat with the last
settings. Each repeat gets a new report directory beside the previous one.
Ctrl+C during a run also drains it. `--listen 127.0.0.1:0` selects
a free port for either the fixture or the dashboard.

For CI, leave the fixture running and use a **new** output directory:

```sh
cargo run --locked -- validate --config examples/local.toml
cargo run --locked -- run --config examples/local.toml --headless --report out/smoke
```

Open `out/smoke/index.html` directly, even offline. Without `--report`, Starling
creates a unique directory under `out/`. An existing report is never overwritten.
Handshake, discovery, transport, assertion, tool, or persistence failures produce
a nonzero exit; a failed workload still gets a report if storage is writable.

### YAML and CLI

`--config` accepts [YAML](examples/local.yaml) (`.yaml` or `.yml`) as
well as [TOML](examples/local.toml). Both examples target the built-in
fixture's `echo` and `status` tools. Both formats use the same fields
and limits. Override `--duration`, `--users`, `--threads`, `--wait`, or
`--timeout` without editing the file. `validate` prints the effective values
without sending any requests.

```sh
cargo run --locked -- validate --config examples/local.yaml --duration 6h --users 4
cargo run --locked -- run --config examples/local.yaml --duration 6h --users 4 --headless
```

For browser control, replace `--headless` with `--web` (if a dashboard is
already running, add `--listen 127.0.0.1:0`). Durations are finite, from
**1 second to 7 days**; the examples default to 30 seconds. Stop early
with Ctrl+C or **Stop run** to drain and save a partial report. Reports are
written at the end, not checkpointed during a run.

To try your own local server, copy either example to a different filename in
`examples/`, then change the target URL, tools and assertions. Git ignores
these local copies and all reports under `out/`.

## Configure a workload

[The example](examples/local.toml) runs two independent sessions for 30 seconds.
Each initializes, requires the tools capability, discovers the configured tools
with `tools/list`, then chooses between `echo` and `status` in a 3:1 ratio.
Selections are seeded per user; response time determines how many calls fit in
the run. Each user waits **after** each call. There are no overlapping task calls
within a session.

- Set `target.url` to a loopback HTTP endpoint you control. Redirects and
  proxies are disabled. Non-loopback targets, embedded credentials and URL
  queries are rejected.
- `target.protocol = "2025-11-25"` selects the `initialize` handshake.
  `"auto"` (default) lets the official `rmcp` SDK attempt `server/discover`
  for 2026-07-28 and fall back to the legacy handshake. A target that needs the
  SDK's full 10-second discovery timeout should use a longer `load.timeout` or
  select the legacy protocol explicitly.
- `load.users` sets concurrency (1–8 independent sessions). `load.threads`
  sets the number of Tokio worker threads in **Starling's load generator**
  (1–8, default 2); it does not change the MCP server's thread pool. Many
  users can share one thread while their requests wait for I/O.
- `run.duration` accepts 1s–7d, `load.wait` 200ms–60s and `load.timeout`
  50ms–15s. The same limits apply in the dashboard and in headless mode.
  Reports record the effective settings. A transport loss or timeout stops the
  run rather than retrying the workload.
- Each `[[tasks]]` entry supplies a unique `tool`, positive `weight`, optional
  `arguments` object and optional `assert_exists` JSON Pointer into the MCP
  result. `isError: true` fails even if the pointer exists. The fixture's
  `reject` tool returns such a result.

## Read the measurements

Setup appears as `initialize` (or `session/startup` in auto mode) and
`tools/list`. Tool-call KPIs exclude both. The operations table separates
transport, protocol, JSON-RPC, capability, tool-result, assertion, timeout and
cancellation outcomes. A tool call's latency includes its assertion, but not
think time.

Timeline buckets use monotonic completion times. Runs up to 30 minutes use
one-second buckets; longer runs use wider intervals to retain at most 1,800
planned points, plus one drain interval. For example, a 6-hour run uses
12-second intervals. Live totals and the partial interval still update each
second; partial updates are not counted twice. The final partial bucket
keeps its actual duration, and empty buckets have no latency percentile.
Overall percentiles come from merged HDR histograms, **not an average of tool
percentiles**. WebSocket subscribers get an initial snapshot and numbered
updates; reconnects and skipped updates get a fresh snapshot.

Reports include the standalone HTML dashboard, public run metadata (`run.json`),
counters and percentiles (`summary.json`), interval data (`timeseries.json`),
`operations.csv`, `errors.csv`, and aggregated `junit.xml`. Reports and dashboard
payloads do not include arguments, raw results, session IDs, full target paths,
or server error text. Run and tool names are public labels.

## Development

The private Rust binary contains the CLI, MCP adapter and fixture, runner,
metrics, Axum controller and report writer in [src](src/). React, TypeScript and
ECharts live in [web](web/). Rebuild the frontend before rebuilding Rust after a
UI change. The Rust toolchain, Cargo dependencies and npm dependencies are pinned
by the toolchain file and lockfiles.

The header uses a [bird-only crop](web/src/assets/starling-bird.webp) of the
[original logo](assets/starling_logo.png). It is embedded in the dashboard so
exported HTML reports display it offline.

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked --release
```

To run the browser test, also build the debug binary (`cargo build --locked`),
then from `web/` run `npx playwright install chromium` and `npm run test:e2e`.
Alternatively, set `STARLING_BROWSER_CHANNEL=msedge` to use an installed Edge.
The test starts its own fixture and dashboard on ephemeral loopback ports,
stops a run from the browser and reopens its report without a network connection.

The Rust workload SDK, stdio transport, staged/open load models, pause/resume,
remote agents, SQLite history, SLO thresholds and soak qualification remain
later milestones in the [specification](docs/SPEC.md).
