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

[Bird quickstart](QUICKSTART.md) · [Example YAML](examples/local.yaml) · [Developer guide](developer.md) · [MIT license](LICENSE)

## Quickstart

With the [CLI installed](QUICKSTART.md#install-the-cli), start from any writable
directory. No config, credentials, or external MCP service are needed:

```sh
starling demo --web --birds 2
```

Open the printed **Dashboard:** URL and click `bird-01` under **The flock**.
Its request count and p95 come from that worker process. Both birds start
their own 15-second fixture runs immediately. Seeing **2 reporting, 0 running**
afterward means both finished and are still reporting. **Start run** controls
a *separate* workload on the dashboard host; it does not start the birds.

## Live flock

Expand **Connect a bird** for the `starling bird --id ... --home ...` command
for *this* dashboard. Run it in another terminal to join idle, append `--demo`
to exercise that bird's own fixture, or append `--config questions.yaml` to
run your scenario on that bird. Each bird writes its own report.

- [Walk through joining, running YAML, disconnecting, and finding reports](QUICKSTART.md#join-a-bird-yourself).
- [Understand heartbeats or connect birds across hosts using mutual TLS](developer.md#connect-across-machines).

### Linux container

The [container quickstart](QUICKSTART.md#linux-container) produces an offline
report without publishing a port. To test two independent containerized birds
checking in over gRPC, use the [developer walkthrough](developer.md#verify-two-linux-containers).

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

The [developer guide](developer.md) covers the build order, the path from a
bird's gRPC heartbeat to the browser, and the Rust, browser, and container
checks used to verify it.

The Rust workload SDK, stdio transport, staged/open load models, pause/resume,
remote job scheduling, cross-worker histogram merging, SQLite history, SLO thresholds and soak qualification remain
later milestones in the [specification](docs/SPEC.md).

## License

Starling is distributed under the [MIT License](LICENSE).
