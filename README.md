<p align="center">
  <a href="assets/starling_logo.png">
    <picture>
      <source media="(prefers-color-scheme: light)" srcset="web/src/assets/starling-bird.webp">
      <img src="assets/starling_logo.png" width="180" alt="Starling bird in flight">
    </picture>
  </a>
</p>

# Starling

Load-test a **local MCP server** and see which tools slow down or fail. Watch
throughput, latency, and errors in the browser, then reopen the same dashboard
as an offline HTML report.

[Quickstart](#quickstart) · [Example YAML](examples/local.yaml) · [Live flock](#live-flock) · [MIT license](LICENSE)

## Quickstart

Try the CLI from any directory. The built-in demo needs no config file, API
key, or separate MCP server.

**Build locally** with Git, [rustup](https://rustup.rs/),
[Node.js](https://nodejs.org/) 22.12+ with npm, and a native linker. On macOS
the Xcode Command Line Tools provide the linker (`xcode-select --install`);
on Debian/Ubuntu use `build-essential`. Rustup selects the pinned toolchain.
Or [build in a Linux container](#linux-container) without installing Rust or Node on the host.

**1. Install `starling`.** Build the dashboard before the Rust binary, which
embeds it. Node is not needed at runtime.

```sh
git clone https://github.com/monk-eee/starling.git
cd starling
npm --prefix web ci
npm --prefix web run build
cargo install --path . --locked
starling --version
```

If your shell can't find `starling`, add Cargo's bin directory to `PATH`
(by default, `export PATH="$HOME/.cargo/bin:$PATH"` in sh, bash, or zsh).

**2. Run a five-second demo.** Starling starts its own local fixture on a free
port, calls `echo` and `status`, saves a report, and shuts the fixture down:

```sh
starling demo --duration 5s
```

The last line gives the tool-call count, failures, and the path to the
standalone HTML report under `out/`. Open it in your browser, even offline.

**3. Try the live dashboard.** Still just one terminal:

```sh
starling demo --web
```

Open the **Dashboard:** URL it prints (not the fixture URL). Click **Start
run** to watch two users call the tools for 30 seconds, or change **Load
settings** first. **Stop run** drains outstanding calls and saves early.
Choose **Export HTML** for an offline copy or **Configure next run** to try
another load. Press Ctrl+C to close the dashboard and its fixture together.

## Live flock

```sh
starling demo --web --birds 2
```

Open the printed **Dashboard:** URL. Each bird in the flock is a separate
process, not a thread or a virtual user. Select one to see its PID, last
heartbeat, sessions, in-flight work, requests, errors and p95. The two
demo birds each exercise their **own** local fixture (one user for 15 seconds),
then stay connected with their final results. Their individual reports are in
`birds/` under the initial controller report directory. They are not sending load to
the dashboard's selected target. Ctrl+C in the controller stops its birds.

To join independently, start a dashboard and use its **Bird telemetry:** URL:

```sh
starling run --config starling.yaml --web --bird-listen 127.0.0.1:8788
# Another terminal: an idle bird checks in but sends no workload requests.
starling bird --id wing-1 --home http://127.0.0.1:8788
# A bird can instead run one locally chosen workload and report its progress:
starling bird --id wing-2 --home http://127.0.0.1:8788 --config starling.yaml
```

Birds send cumulative counters over gRPC once per second; the browser receives
roster snapshots over WebSocket. Five seconds without a check-in marks a bird
**stale**, not failed. Reconnection does not double-count; an active ID cannot
be claimed by a second process. Stale entries expire after five minutes. Per-bird
p95s describe independent runs; the page does not average them or mix bird
traffic into the local run charts. No targets, questions, or raw results are
transmitted by the telemetry service. It has no job-dispatch or start RPC.

**Across machines or containers:** expose only the separate gRPC listener on
your private network, not the dashboard. Set `STARLING_FLOCK_CA`,
`STARLING_FLOCK_CERT`, and `STARLING_FLOCK_KEY` to PEM **file paths** on the
controller and each worker. Use a dedicated CA, a server certificate valid for
the controller's DNS name, and a distinct client certificate on each worker.
Start the controller with `--web --bird-listen <private-IP>:8788`; use
`--home https://<controller-DNS>:8788` on the workers. Non-loopback telemetry
requires mutual TLS. Plain HTTP is permitted only on numeric loopback. Workload
endpoints remain local to the bird running them. Do not commit keys or workloads
containing private inputs.

A bird with a workload stays connected after finishing so its final measurements
remain visible. If it loses contact, it retries reporting, not requests to the
test target. After 30 seconds without an acknowledgement it drains and exits.

### Linux container

From the clone, build the CLI and run the same demo with no external network:

```sh
docker build -t starling .
mkdir -p out
docker run --rm --network none --user "$(id -u):$(id -g)" \
  -v "$PWD/out:/work/out" starling demo --duration 5s
```

Open the generated `out/run-…/index.html` on the host. The fixture and CLI
share the container's loopback interface; `127.0.0.1` inside a container
is **not** the host's loopback. For a separate local MCP service, run the CLI
in the same network namespace as that service. The headless demo needs no
published ports.

To end a long run early, use `docker stop --timeout 45 CONTAINER` in another
terminal (replace `CONTAINER` with its name or ID). On Linux and macOS, Starling
handles SIGTERM like Ctrl+C: it stops issuing requests, waits for in-flight work
and session shutdown, then writes the report. Allow enough time for both the
configured operation timeout and cleanup. A forced kill cannot save the report.

### Create your own workload

In the directory where you want to keep your workload:

```sh
starling init
starling validate
```

`init` creates the [example YAML](examples/local.yaml) and never overwrites a file.
Edit its target, methods, questions and assertions for a local MCP server you control. To exercise the
unchanged starter instead, run `starling fixture` in a second terminal.
Then run `starling run` for a headless test or `starling run --web` for the
dashboard (default `http://127.0.0.1:8787`). `validate` checks the file
without sending any requests.

Each run gets a fresh report directory. Pass `--report PATH` to choose your
own; Starling will not overwrite an existing report.
Handshake, discovery, transport, assertion, tool, or persistence failures produce
a nonzero exit; a failed workload still gets a report if storage is writable.

### YAML and CLI

`--config` accepts [YAML](examples/local.yaml) (`.yaml` or `.yml`) as
well as [TOML](examples/local.toml). The YAML shows two questions for one
tool, a measured catalog request, and an MCP prompt. Both formats use the same fields
and limits. Override `--duration`, `--users`, `--threads`, `--wait`, or
`--timeout` without editing the file. `validate` prints the effective values
without sending any requests.

```sh
starling init workload.toml
starling validate --config workload.toml --duration 6h --users 4
starling run --config workload.toml --duration 6h --users 4 --headless
```

For browser control, replace `--headless` with `--web` (if a dashboard is
already running, add `--listen 127.0.0.1:0`). Durations are finite, from
**1 second to 7 days**; the examples default to 30 seconds. Stop early
with Ctrl+C or **Stop run** to drain and save a partial report. Reports are
written at the end, not checkpointed during a run.

To try your own local server, copy either example to a different filename in
`examples/`, then change the target URL, tools and assertions. Git ignores
these local copies and all reports under `out/`.

**From the dashboard:** choose **Download example YAML**, edit it, then use
**Load YAML**. In the demo the download points to its actual fixture port.
Loading validates the file and previews the request labels; it does **not**
contact the target. Click **Start run** when ready. A running workload cannot
be replaced. Loading after a finished run reserves a fresh report.

## Configure a workload

[The example](examples/local.toml) runs two independent sessions for 30 seconds.
Each initializes, requires the tools capability, discovers the configured tools
with `tools/list`, then chooses between `echo` and `status` in a 3:1 ratio.
Selections are seeded per user; response time determines how many calls fit in
the run. Each user waits **after** each call. There are no overlapping task calls
within a session.

- Set `target.url` to a loopback HTTP endpoint you control. Redirects and
  proxies are disabled. Non-loopback targets, embedded credentials and URL
  queries are rejected. Optionally add `target.endpoints` as a mapping of
  public aliases to alternative local URLs. Choose **one endpoint per run**
  with `target.select: candidate` in YAML, `--endpoint candidate` on the CLI,
  or the dashboard's Endpoint selector; `default`
  uses `target.url`. The same validation applies to every alternative.
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
- Each task has a positive `weight` and may have a public `name` and an
  `assert_exists` JSON Pointer into the result. Give distinct names to repeated
  cases for the same tool. Put the **question in `arguments`**, using the
  argument name from the target's schema; Starling does not generate an LLM answer.

| `method` | Request fields | What is measured |
|---|---|---|
| `tools/call` (default) | `tool`, optional `arguments` | One actual tool invocation; `isError: true` fails independently of assertions |
| `tools/list` | None | First page of the tool catalog, without using the discovery cache |
| `prompts/get` | `prompt`, optional string-valued `arguments` | Retrieve the named MCP prompt |
| `prompts/list` | None | First page of the prompt catalog, without using the discovery cache |

These are MCP methods, not arbitrary HTTP verbs. The target must advertise
the capabilities required by the workload.

## Read the measurements

Setup appears as `initialize` (or `session/startup` in auto mode) and
`tools/list` discovery when a tool is called. Workload KPIs exclude setup
and include all scheduled methods. For example `tools/list` is setup, while
`task/tools/list/catalog` is a scheduled request. The operations table separates
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
or server error text. Run, task, prompt, tool and endpoint alias names are public labels.

## Development

The Rust binary contains the CLI, MCP adapter and fixture, runner,
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
Alternatively, set `STARLING_BROWSER_CHANNEL` to an installed Playwright
Chromium channel such as `chrome`.
The test starts its own fixture and dashboard on ephemeral loopback ports,
stops a run from the browser and reopens its report without a network connection.

The Rust workload SDK, stdio transport, staged/open load models, pause/resume,
remote job scheduling, cross-worker histogram merging, SQLite history, SLO thresholds and soak qualification remain
later milestones in the [specification](docs/SPEC.md).

## License

Starling is distributed under the [MIT License](LICENSE).
