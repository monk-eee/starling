<p align="center">
  <a href="web/src/assets/starling-bird.webp">
    <img src="web/src/assets/starling-bird.webp" width="180" alt="Starling bird in flight">
  </a>
</p>

# Starling

Load-test a **local MCP server** and see which tools slow down or fail. Watch
throughput, latency, and errors in the browser, then reopen the same dashboard
as an offline HTML report. For compatibility rather than load, run a
[one-shot or distributed MCP conformance check](#check-mcp-conformance).

[Install and verify the CLI](QUICKSTART.md#install-the-cli) · [Bird quickstart](QUICKSTART.md#launch-the-flock) · [Example YAML](examples/local.yaml) · [Developer guide](developer.md) · [MIT license](LICENSE)

[Architecture decisions](docs/adr.d/README.md) · [Changelog](CHANGELOG.md) · [Contributing and pre-commit](CONTRIBUTING.md) · [Test coverage](developer.md#measure-test-coverage) · [Report a security issue](SECURITY.md)

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

## Check MCP conformance

```sh
starling check --demo --all-protocols                    # one process, four revisions
starling collect --demo-birds 2                          # two independent check workers, same matrix
starling check --url http://127.0.0.1:8081/mcp --all-protocols
starling check --config questions.yaml --protocol 2025-11-25,2026-07-28
```

In the ready dashboard, **Check revision** or **Check all revisions** shows
the evidence and lets you download JSON without starting a load run. Each
revision gets a fresh session. The versioned `mcp.streamable-http.core.v1`
suite checks lifecycle, the first page of each advertised tool/prompt/resource
catalog, and client transport cleanup. It never calls a tool or reads a
resource. The modern revision also checks for complete-result caching fields.
The [rule reference](developer.md#extending-the-conformance-suite) explains
exactly what those checks establish.

A one-shot CLI run writes `conformance.json`; the finite collector writes
`fleet-conformance.json` with the expected and missing birds and each worker's
unmodified findings. `pass` means **the applicable checks in this profile
passed**, not that an entire server or every feature is certified. Optional
capabilities not advertised are `skip`, a refused revision is `unsupported`,
and a timeout is `inconclusive`. Non-passing campaigns exit nonzero. See the
[distributed walkthrough](QUICKSTART.md#check-a-server-once-or-as-a-fleet).

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
and limits. Override `--protocol`, `--duration`, `--users`, `--threads`,
`--wait`, or `--timeout` without editing the file. `validate` prints the effective values
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
Selections are seeded per user. By default, each user starts its next request
as soon as the previous one finishes. There is no pacing timer and no overlap
within a session. A slower target naturally completes fewer requests.

- Set `target.url` to a loopback HTTP endpoint you control. Redirects and
  proxies are disabled. Non-loopback targets, embedded credentials and URL
  queries are rejected. Optionally add `target.endpoints` as a mapping of
  public aliases to alternative local URLs. Choose **one endpoint per run**
  with `target.select: candidate` in YAML, `--endpoint candidate` on the CLI,
  or the dashboard's Endpoint selector; `default`
  uses `target.url`. The same validation applies to every alternative.
- `target.protocol` sets the [requested MCP revision](#test-an-mcp-protocol-revision).
  An omitted value means `auto`; the bundled examples explicitly request `2025-11-25`.
- `load.users` sets concurrency (1–8 independent sessions). `load.threads`
  sets the number of Tokio worker threads in **Starling's load generator**
  (1–8, default 2); it does not change the MCP server's thread pool. Many
  users can share one thread while their requests wait for I/O.
- `load.wait` is optional. Omit it or set `wait: 0` for **no pause**. To model
  user think time, set a positive duration such as `250ms` (1ms–60s). An
  existing file with a positive wait keeps that behavior; `--wait 0` or **0**
  in the dashboard overrides it for the run.
- `run.duration` accepts 1s–7d and `load.timeout` 50ms–15s. The same limits
  apply in the dashboard and in headless mode.
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

### Test an MCP protocol revision

Select **MCP protocol revision** under **Load settings** before starting a
dashboard run, or set `target.protocol` in your workload:

```yaml
target:
  url: http://127.0.0.1:8081/mcp
  protocol: "2025-06-18"
```

| Selection | What Starling does |
|---|---|
| `auto` | Tries `server/discover` for `2026-07-28`. On the SDK's legacy fallback it offers `2025-11-25` in `initialize` and can accept an earlier supported revision from the server. |
| `2026-07-28` | Requires `server/discover` and uses per-request protocol metadata. No `initialize` fallback. |
| `2025-11-25`, `2025-06-18`, `2025-03-26` | Sends `initialize` with exactly the selected revision. A different reply fails startup before workload requests are sent. |

Use **Configure next run** to repeat the same workload with a different
revision. The choice overrides the loaded YAML for that run, not the file.
Headless equivalent: `starling run --config questions.yaml --protocol 2025-06-18`.
For a self-contained check, try `starling demo --protocol 2026-07-28 --duration 2s`.
`validate` checks the selection without contacting the target. Auto's SDK
discovery timeout is 10 seconds; a shorter workload timeout can expire before fallback.

The summary distinguishes `run.protocol` (requested) from `protocols`
(revisions recorded for established sessions). `run.protocol_revisions` is
the list of available choices, not a list of tests that passed. A successful run demonstrates
that **your configured methods** worked with that revision, not full MCP
conformance. The transport is still loopback **Streamable HTTP**; stdio and
the `2024-11-05` HTTP+SSE transport are not selectable.

## Read the measurements

Setup appears as `initialize`, `server/discover`, or `session/startup` (auto), and
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

## Structured logs and OpenTelemetry

Starling writes lifecycle events as JSON lines on stderr. No external
collector is contacted by default; `STARLING_TELEMETRY=none` disables those
logs, not CLI diagnostics or the live flock. To see the events and check that
the HTML report was written, try the [one-second smoke test](QUICKSTART.md#verify-the-install-and-read-the-logs).

To also export logs, run spans, and workload metrics to your own OTLP/HTTP
collector, first build with `--features otlp`, then select it at runtime.
For example, from a checkout with the dashboard built and a local collector
already listening on port 4318:

```sh
STARLING_TELEMETRY=stderr,otlp OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318 \
  cargo run --locked --features otlp -- demo --duration 2s
```

That address is the **collector**, not the MCP target or the birds' `--home`.
It receives counts by outcome and latency histograms, not workload arguments
or response bodies. See [how logging starts and stops](developer.md#how-logging-starts-and-stops)
and [export settings](developer.md#process-telemetry).

## Development

The [developer guide](developer.md) covers the build order, the path from a
bird's gRPC heartbeat to the browser, and the Rust, browser, and container
checks used to verify it.

See [known gaps](developer.md#known-gaps) before treating a
[roadmap requirement](docs/SPEC.md) as a shipped feature.

## License

Starling is distributed under the [MIT License](LICENSE).
