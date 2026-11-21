# Starling Technical Specification

**Status:** Draft v0.1

**Date:** 19 November 2026

**Product:** Starling — a Rust load-testing and performance-analysis platform for Model Context Protocol servers

**Read this as a roadmap, not a CLI reference.** The current CLI runs YAML or
TOML scenarios against a loopback Streamable HTTP target. Birds report their
own runs over gRPC; the controller does not assign them work. The
[bootstrap guide](../QUICKSTART.md#install-the-cli) builds one Rust binary
with an embedded dashboard. [Process logging](../developer.md#process-telemetry)
is local by default, with optional OTLP/HTTP export. Use the
[workload reference](../README.md#configure-a-workload) for supported methods
and limits, [known gaps](../developer.md#known-gaps) for unimplemented goals,
and the [ADR index](adr.d/README.md) for decisions under review.

## 1. Executive summary

The proposed platform is a code-first MCP load tester for local and distributed workloads, with live results in a browser. The current CLI uses workload files, not a Rust workload SDK.

The design draws on these ideas from Locust:

- User behavior defined in code.
- Weighted tasks, per-user setup/teardown, and configurable think time.
- Live control and visualization through a web interface.
- Controller/worker distribution for larger tests.
- Headless execution for CI.

Locust creates one simulated-user object per virtual user, executes weighted tasks sequentially for that user, and waits between tasks. Its distributed mode assigns user execution to workers while the controller hosts the web interface and aggregates statistics. [[docs.locust.io]](https://docs.locust.io/en/stable/writing-a-locustfile.html), [[docs.locust.io]](https://docs.locust.io/en/stable/running-distributed.html)

Starling adapts that model to MCP-specific concerns:

- MCP initialization and capability negotiation.
- Stateful client sessions.
- Tools, resources, prompts, subscriptions, notifications, and long-running tasks.
- stdio and Streamable HTTP transports.
- JSON-RPC failures versus transport failures versus semantic tool failures.
- Stream timing, progress messages, reconnects, and server-to-client requests.
- Per-tool, per-method, per-transport, and per-scenario statistics.

The backend, load engine, controller, workers, and CLI will be Rust. The frontend should be React/TypeScript compiled into static assets and embedded in the controller binary. Using a browser framework here provides faster dashboard development without weakening the requirement that the performance-sensitive system be Rust.

---

## 2. Product goals

### 2.1 Primary goals

1. **Load test MCP servers directly**

- Exercise MCP protocol operations without requiring an LLM.
- Support local stdio servers and remote Streamable HTTP servers.
- Measure initialization, discovery, invocation, streaming, and shutdown independently.
2. **Model realistic MCP clients**

- Maintain stateful sessions.
- Run sequential workflows within a user while allowing concurrency between users.
- Support weighted tasks, think time, setup, teardown, input generation, and assertions.
3. **Provide a first-class frontend**

- Configure and control runs.
- Display live graphs and statistics.
- Drill into tools, errors, workers, sessions, and sampled requests.
- Compare current results with a previous baseline.
4. **Scale horizontally**

- Run multiple agents on one machine or many machines.
- Keep the controller out of the hot request path.
- Merge counters and histograms without averaging percentiles.
5. **Work in development and CI**

- Interactive browser mode for developers.
- Headless mode with exit codes and SLO gates.
- JSON, CSV, JUnit XML and HTML reports; optional OpenTelemetry export.
  Native Prometheus output is a roadmap goal, not a current CLI option.

### 2.2 Secondary goals

- Fault injection for transport interruptions, delays, malformed messages, and server restarts.
- Protocol conformance testing alongside performance testing.
- Reusable result bundles for offline viewing.
- Extension points for custom MCP transports and metrics sinks.

### 2.3 Non-goals for v1

- Generating realistic LLM completions.
- Replacing a general HTTP load tester.
- Operating as a hosted multi-tenant SaaS.
- Browser-driven testing.
- Profiling the MCP server’s internal code unless it exports telemetry.
- Automatically deciding whether a tool result is semantically correct without user assertions.

---

## 3. MCP baseline

The [MCP adapter](../src/mcp.rs) uses the [public Rust SDK](https://github.com/modelcontextprotocol/rust-sdk).
The workload or ready dashboard can select automatic negotiation or pin an
exact revision over Streamable HTTP. The [implemented profiles](../README.md#test-an-mcp-protocol-revision)
use `initialize` for the supported 2025 revisions and `server/discover` for
2026-07-28. Selecting a revision is not a full conformance test.
The separate [core conformance suite](../developer.md#extending-the-conformance-suite)
evaluates named lifecycle and catalog checks per revision and writes evidence.

The [MCP architecture](https://modelcontextprotocol.io/specification/2025-11-25/architecture)
separates JSON-RPC operations from their transport. The design distinguishes:

- **Logical MCP operation metrics**, such as `tools/call`.
- **Physical transport metrics**, such as HTTP requests, SSE reconnects, bytes, and connection setup.

The first supported transports will be:

1. **stdio**

- Starling launches the MCP server process.
- JSON-RPC messages travel over stdin/stdout.
- stderr is captured as server logging, not automatically classified as failure.
2. **Streamable HTTP**

- POST-based JSON-RPC exchange.
- JSON or SSE responses.
- Optional GET SSE stream for server-to-client communication.
- Session ID, reconnect, redelivery, and cancellation handling.

These are the two standard transport families defined in the referenced MCP transport specification. [[modelconte...rotocol.io]](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports)

Legacy HTTP+SSE support should be a post-MVP compatibility module rather than part of the core engine.

---

## 4. User experience

The commands in this section illustrate the **proposed** interface. For
commands you can run today, start with the [quickstart](../QUICKSTART.md).

### 4.1 Operating modes

#### Interactive mode

Shell

starling run ./tests/search_workload \

  --target https://mcp.example.com/mcp \

  --web

Behavior:

1. Builds or loads the workload.
2. Starts the controller and local agent.
3. Opens the dashboard.
4. Lets the user configure concurrency, arrival rate, duration, and stages.
5. Streams live metrics until the run is stopped.
6. Persists and displays the final report.

#### Headless mode

For the implemented headless gate use the top-level workload
[`assert` mapping](../README.md#use-conformance-and-performance-as-deployment-gates).
There is no `--fail-on-slo` flag; the more general selectors, stage gates and
flags below are proposals.

Shell

starling run ./tests/search_workload \

  --config load.toml \

  --headless \

  --report out/starling-report \

  --fail-on-slo

The process exits nonzero when:

- An SLO gate fails.
- The target cannot initialize.
- Required MCP capabilities are missing.
- A configured maximum generator-lag or worker-loss threshold is exceeded.
- The test infrastructure itself fails.

#### Distributed mode

Shell

starling controller --listen 0.0.0.0:8787

 

starling agent \

  --controller https://controller.example.com:8787 \

  --token-env STARLING_AGENT_TOKEN

The controller owns configuration, orchestration, aggregation, storage, and the UI. Agents own virtual users, MCP sessions, operation execution, and local metric aggregation.

### 4.2 CLI commands

Command | Purpose

`starling init` | Create a workload project from a template

`starling validate` | Compile and validate configuration, schemas, targets, and capabilities

`starling run` | Start a local or controller-backed run

`starling controller` | Start the control plane and dashboard

`starling agent` | Register a load-generating worker

`starling report` | Render an HTML report from a saved run

`starling compare` | Compare a run against a baseline

`starling inspect` | Initialize a target and display its capabilities

`starling doctor` | Check OS limits, certificates, clocks, CPU, and file descriptors

---

## 5. Frontend specification

The dashboard is a required product surface, not an optional observability add-on.

### 5.1 Frontend stack

- React
- TypeScript
- Vite
- TanStack Query for REST state
- Zustand or Redux Toolkit for live-run state
- Apache ECharts using canvas rendering
- WebSocket for live metrics
- REST for configuration, history, errors, and exports
- Playwright for end-to-end tests

The compiled frontend will be embedded into the Rust controller using `rust-embed` or equivalent. A default installation should remain a single executable.

### 5.2 Navigation

1. **Runs**

- Active, completed, aborted, and failed runs.
- Name, target, start time, duration, peak users, throughput, p95, error rate, and SLO status.
2. **New Run**

- Workload selection.
- Target and transport.
- Authentication profile.
- Load shape.
- Duration and warm-up.
- SLO gates.
- Worker selection.
- Advanced MCP options.
3. **Live Dashboard**

- Primary operational view.
4. **Operations**

- Per-method and per-tool tables.
5. **Errors**

- Grouped error fingerprints and samples.
6. **Workers**

- Agent health and generator saturation.
7. **Request Samples**

- Redacted trace samples.
8. **Comparison**

- Current run versus baseline.
9. **Settings**

- Retention, redaction, telemetry, defaults, and access control.

### 5.3 Live dashboard layout

#### Header

- Run name and status.
- Target and negotiated MCP version.
- Elapsed and remaining time.
- Connected/desired workers.
- Start, pause, resume, stop, and abort controls.
- Export report button.
- SLO status indicator.

#### KPI cards

- Active virtual users.
- Active MCP sessions.
- Operations per second.
- Successful operations per second.
- Error rate.
- p50, p95, and p99 logical operation latency.
- Generator scheduling lag.
- In-flight operations.

#### Required graphs

Graph | Data

Throughput | Successful, failed, cancelled, and timed-out operations per second

Latency percentiles | p50, p75, p90, p95, p99, and max

Latency distribution | Histogram or time-bucketed heat map

Error rate | Failures by category over time

Load | Target users, active users, active sessions, and in-flight operations

Generator lag | Scheduled-versus-actual dispatch delay

MCP operation mix | Calls split by `tools/call`, `resources/read`, `prompts/get`, etc.

Tool activity | Top tools by rate, p95, and failures

Transport health | Connections, reconnects, SSE streams, timeouts, and bytes

Worker utilization | CPU, memory, open connections, and generated operations

Long-running tasks | Acknowledgement latency and completion latency

Progress activity | Progress notifications and time-to-first-progress

All time-series graphs must support:

- Synchronized crosshair.
- Zoom and pan.
- Run-phase annotations.
- Worker join/leave markers.
- Error-spike markers.
- Selection by tool, scenario, method, worker, transport, and outcome.
- Comparison overlay against a baseline.
- Download as PNG and CSV.

### 5.4 Operations table

Columns:

- Operation or tool.
- Count.
- Operations per second.
- Success rate.
- Error rate.
- Mean.
- Median.
- p90.
- p95.
- p99.
- Min/max.
- Average request/response bytes.
- Current in-flight count.

Rows can be grouped by:

- MCP method.
- Tool name.
- Scenario.
- Transport.
- Result type.
- Worker.

Tool arguments must never become metric labels because that would create unbounded cardinality and could leak data.

### 5.5 Error explorer

Errors are grouped by a stable fingerprint:

Plain Text

category + MCP method + tool + normalized error code + normalized message

Categories:

- Connection
- TLS
- Authentication
- HTTP
- SSE
- JSON-RPC protocol
- MCP capability
- MCP application error
- Tool result with `isError`
- Timeout
- Cancellation
- Assertion
- Invalid schema
- Generator
- Worker infrastructure

Each group displays:

- First and latest occurrence.
- Count and rate.
- Affected tools and scenarios.
- Affected workers.
- Redacted request and response samples.
- Correlated server logs when available.

### 5.6 Live data behavior

- Agents aggregate locally into one-second intervals.
- Controller broadcasts at most one dashboard update per second by default.
- WebSocket messages are sequence-numbered.
- A reconnecting browser requests a fresh snapshot and then resumes deltas.
- The frontend retains high-resolution data for the visible run.
- The backend compacts older data into 10-second and 60-second buckets.
- Chart rendering must not affect load generation because the browser and controller are outside the worker hot path.

---

## 6. System architecture

Plain Text

                         ┌──────────────────────────┐

                         │      Web Browser         │

                         │ React + TypeScript       │

                         │ Controls, graphs, drill  │

                         └────────────┬─────────────┘

                                      │ REST + WebSocket

                         ┌────────────▼─────────────┐

                         │   Starling Controller    │

                         │ Axum API                 │

                         │ Run scheduler            │

                         │ Metrics aggregator       │

                         │ SLO evaluator            │

                         │ Report/storage service   │

                         └───────┬────────┬─────────┘

                                 │        │

                      gRPC/TLS   │        │ SQLite / report bundle

                    ┌────────────▼─┐   ┌──▼──────────────┐

                    │ Agent A      │   │ Run Store       │

                    │ Tokio runner │   │ Metadata        │

                    │ MCP clients  │   │ Time series     │

                    └──────┬───────┘   │ Errors/samples │

                           │           └─────────────────┘

               ┌───────────┴───────────┐

               │                       │

       ┌───────▼────────┐      ┌──────▼───────────┐

       │ stdio MCP      │      │ Streamable HTTP  │

       │ subprocesses   │      │ MCP server       │

       └────────────────┘      └──────────────────┘

### 6.1 Rust workspace

The layout below is a proposed split, **not** a set of repositories to clone.
Today the [CLI package](../Cargo.toml) and [dashboard](../web/) build from this
checkout using their respective lockfiles.

starling/

  crates/

    starling-cli/

    starling-sdk/

    starling-core/

    starling-mcp/

    starling-runner/

    starling-metrics/

    starling-controller/

    starling-agent/

    starling-storage/

    starling-report/

    starling-protocol/

    starling-test-server/

  web/

    src/

    tests/

### 6.2 Core technology choices

Area | Choice

Async runtime | Tokio

MCP implementation | Official `rmcp` SDK behind a Starling adapter

HTTP server | Axum

HTTP client | Hyper/Reqwest, preferably through `rmcp`

Controller-agent RPC | Tonic gRPC

Serialization | Serde

Metrics histogram | Mergeable HDR histograms

Local persistence | SQLite in WAL mode

Report time series | Compressed columnar or chunked JSON data

Logging/tracing | Structured stderr events; optional OTLP/HTTP logs, run spans and request metrics ([current behavior](../developer.md#process-telemetry))

CLI | Clap

Configuration | TOML with environment-variable substitution

Frontend | React, TypeScript, ECharts

Starling should wrap the MCP SDK instead of exposing it directly to workloads. That allows Starling to instrument lifecycle phases, maintain stable APIs, and pin protocol compatibility independently.

---

## 7. Workload model

### 7.1 Core concepts

- **Run:** One execution of a workload.
- **Scenario:** A class of simulated MCP user.
- **Virtual user:** One independent scenario instance.
- **Session:** One logical MCP client-server session.
- **Task:** A weighted unit of user behavior.
- **Operation:** One measured MCP or custom action.
- **Stage:** A time-bounded load target.
- **Assertion:** A correctness rule applied to a response.
- **Load shape:** The function determining user or arrival rate over time.

### 7.2 Code-first Rust API

The proposed authoring experience is a Rust crate that links against `starling-sdk`. This would avoid unstable dynamic-library loading and give workloads normal Cargo dependencies, type checking, IDE support, and reusable helper modules. For an executable scenario today, use the [YAML example](../examples/local.yaml).

Illustrative proposed API:

Rust

use serde_json::json;

use starling::prelude::*;

 

async fn initialize(user: &mut User) -> Result<()> {

    user.mcp()

        .require_capability(ServerCapability::Tools)

        .await?;

 

    Ok(())

}

 

async fn search(user: &mut User) -> Result<()> {

    let query = user.data().random_search_query();

 

    let result = user

        .operation("search-tool")

        .call_tool("search", json!({

            "query": query,

            "limit": 10

        }))

        .timeout(Duration::from_secs(10))

        .send()

        .await?;

 

    result

        .assert()

        .not_tool_error()?

        .json_path("$.content[0]")?

        .exists()?;

 

    Ok(())

}

 

async fn read_resource(user: &mut User) -> Result<()> {

    user.operation("read-catalog")

        .read_resource("catalog://products")

        .send()

        .await?

        .assert()

        .content_type("application/json")?;

 

    Ok(())

}

 

fn main() -> Result<()> {

    Starling::builder()

        .scenario(

            Scenario::builder("research-user")

                .weight(3)

                .on_start(initialize)

                .task(Task::new("search", search).weight(5))

                .task(Task::new("read-resource", read_resource).weight(1))

                .wait(Wait::between(

                    Duration::from_millis(500),

                    Duration::from_secs(2),

                )),

        )

        .run()

}

This mirrors Locust’s useful model of per-user setup, weighted tasks, sequential execution within a user, and wait time, while keeping the workload native to Rust. [[docs.locust.io]](https://docs.locust.io/en/stable/writing-a-locustfile.html)

### 7.3 Configuration

The TOML below illustrates the proposed configuration. Today's YAML/TOML
accepts one `target.protocol` per run, not a `protocol_versions` list;
see the [runnable example](../examples/local.yaml). For current workload-wide
limits use `[assert]` in TOML (`assert:` in YAML), not the proposed `[[slo]]`
per-operation selectors below.

TOML

[run]

name = "search-service-baseline"

duration = "15m"

warmup = "2m"

seed = 48151623

 

[target]

transport = "streamable-http"

url = "https://mcp.example.com/mcp"

protocol_versions = ["2026-07-28", "2025-11-25"]

 

[target.auth]

type = "bearer"

token_env = "MCP_ACCESS_TOKEN"

 

[load]

model = "closed"

 

[[load.stages]]

duration = "2m"

users = 50

spawn_rate = 10

 

[[load.stages]]

duration = "5m"

users = 500

spawn_rate = 50

 

[[load.stages]]

duration = "5m"

users = 1000

spawn_rate = 100

 

[[load.stages]]

duration = "3m"

users = 0

spawn_rate = 100

 

[[slo]]

metric = "operation_latency"

selector = 'tool="search"'

percentile = 95

operator = "<"

value = "500ms"

 

[[slo]]

metric = "error_rate"

operator = "<"

value = 0.01

### 7.4 Load models

#### Closed model

Maintains a target number of concurrent virtual users. Each user executes a task and waits before selecting another task.

Use for:

- Stateful workflows.
- “How many simultaneous clients can the server sustain?”
- Soak testing.

#### Open model

Schedules arrivals at a configured rate independently of response time.

Use for:

- Constant offered load.
- Throughput-limit testing.
- Avoiding the self-throttling behavior of a closed model.

#### Supported shapes

- Constant users.
- Constant arrival rate.
- Ramp.
- Step.
- Spike.
- Soak.
- Custom stage sequence.
- Custom Rust load-shape function.

Starling must record scheduling delay separately from server latency. Otherwise a saturated generator can appear to produce low server latency simply because it failed to send work on time.

### 7.5 Session strategies

Strategy | Behavior | Typical use

Per user | One initialized session per virtual user | Default realistic mode

Per task | Reinitialize for each task | Initialization stress

Per operation | New connection/session for every operation | Connection churn

Shared pool | Users lease initialized sessions | Stateless service throughput

Fixed sessions | N sessions with M operations each | Controlled capacity experiments

For stdio, process-per-user can be very expensive. Starling must expose a subprocess ceiling and warn when the requested user count implies an unsafe number of child processes.

---

## 8. MCP execution semantics

### 8.1 Session lifecycle

Each session proceeds through:

1. Transport connection or subprocess launch.
2. Protocol/version negotiation.
3. Capability exchange.
4. Initialized state.
5. Optional discovery:

- `tools/list`
- `resources/list`
- `prompts/list`
6. Scenario operations.
7. Graceful session close or subprocess termination.

Initialization must be a separately measured operation. Test authors can include it in or exclude it from steady-state statistics.

### 8.2 Supported operations

#### MVP

- Initialize and ping.
- Tools list and call.
- Resources list, read, and subscribe.
- Prompts list and get.
- Pagination.
- Notifications and progress.
- Cancellation.
- Logging notifications.
- Completion requests.
- Roots responses.
- Deterministic handling of sampling and elicitation requests.

#### Post-MVP

- Full long-running task lifecycle.
- Response-cache behavior.
- Multi-round-trip requests.
- Custom transports.
- Legacy HTTP+SSE.
- Fault injection at individual protocol phases.

### 8.3 Server-to-client behavior

Some MCP servers issue requests back to the client. Starling must provide configurable policies:

TOML

[client.sampling]

mode = "fixture"

fixture = "fixtures/sampling-response.json"

 

[client.elicitation]

mode = "generated"

generator = "user-profile"

 

[client.roots]

mode = "static"

uris = ["file:///workspace"]

Supported policies:

- Return a static fixture.
- Generate a deterministic response from the run seed.
- Reject the request.
- Delay the response.
- Time out deliberately.
- Invoke user-defined Rust code.

No real model should be called unless the workload explicitly installs a sampling handler.

### 8.4 Success classification

An HTTP 200 response is not sufficient to mark an MCP operation successful.

Starling distinguishes:

1. Transport completed.
2. JSON-RPC response was valid.
3. No JSON-RPC error was returned.
4. MCP result did not indicate an application/tool error.
5. User assertions passed.

The UI should let users view both:

- **Protocol success rate**
- **Scenario/assertion success rate**

---

## 9. Metrics specification

### 9.1 Timing definitions

For every scheduled operation:

- `scheduled_at`: Intended dispatch time.
- `started_at`: Actual dispatch time.
- `first_byte_at`: First response byte.
- `first_event_at`: First SSE or progress event.
- `acknowledged_at`: Long-running task acknowledgement.
- `completed_at`: Final logical result.

Derived metrics:

Plain Text

generator_queue_delay = started_at - scheduled_at

time_to_first_byte    = first_byte_at - started_at

time_to_first_event   = first_event_at - started_at

service_time          = completed_at - started_at

end_to_end_time       = completed_at - scheduled_at

task_completion_time  = completed_at - acknowledged_at

### 9.2 Dimensions

These are proposed analysis dimensions. Today's OTLP request metrics expose
only the MCP method and outcome as metric attributes, with process identity
on the resource. They do not export tool, scenario or run names.

Bounded labels:

- Run.
- Scenario.
- MCP method.
- Tool name.
- Resource template or normalized URI group.
- Prompt name.
- Transport.
- Worker.
- Outcome.
- Error category.
- Protocol version.
- Test stage.

Prohibited labels:

- Request ID.
- Session ID.
- Raw resource URI.
- Tool arguments.
- User-supplied text.
- Error messages without normalization.

### 9.3 Aggregation

The proposed fleet aggregation protocol needs more than the per-bird p95 in
today's heartbeat. Do not average those values; see the
[measurement ADR](adr.d/0003-percentiles-need-distributions.md).
The proposed aggregate frames contain:

- Exact counters.
- Gauges.
- Mergeable latency histograms.
- Bounded error fingerprints.
- Reservoir-sampled traces.
- Agent resource statistics.

Such a controller would merge histograms, never average worker p95 values.
Today an external collector can combine the exported OTLP histogram buckets;
the live dashboard does not consume that external result.

### 9.4 Generator health

Every run must expose:

- Agent CPU and memory.
- Tokio runtime scheduling delay.
- Generator queue delay.
- Open file descriptors.
- Active connections and subprocesses.
- Dropped metric frames.
- Controller-agent clock offset.
- Controller ingestion delay.

When the load generator is the bottleneck, Starling must display a prominent **generator saturated** warning instead of presenting the run as a valid server capacity result.

---

## 10. Controller API

### 10.1 REST endpoints

Plain Text

GET    /api/v1/health

GET    /api/v1/capabilities

 

GET    /api/v1/workloads

POST   /api/v1/workloads/validate

 

GET    /api/v1/runs

POST   /api/v1/runs

GET    /api/v1/runs/{run_id}

 

POST   /api/v1/runs/{run_id}/start

POST   /api/v1/runs/{run_id}/pause

POST   /api/v1/runs/{run_id}/resume

POST   /api/v1/runs/{run_id}/stop

POST   /api/v1/runs/{run_id}/abort

 

GET    /api/v1/runs/{run_id}/summary

GET    /api/v1/runs/{run_id}/series

GET    /api/v1/runs/{run_id}/operations

GET    /api/v1/runs/{run_id}/errors

GET    /api/v1/runs/{run_id}/samples

GET    /api/v1/runs/{run_id}/workers

GET    /api/v1/runs/{run_id}/export

 

GET    /api/v1/baselines

POST   /api/v1/runs/{run_id}/baseline

GET    /api/v1/compare

### 10.2 WebSocket

Plain Text

GET /api/v1/runs/{run_id}/live

Messages:

JSON

{

  "type": "tick",

  "sequence": 1842,

  "timestamp": "2026-11-19T12:00:00Z",

  "runStatus": "running",

  "phase": "steady-state",

  "kpis": {},

  "series": [],

  "workerChanges": [],

  "sloChanges": []

}

The protocol must support:

- Snapshot request.
- Delta updates.
- Sequence-gap detection.
- Backpressure.
- Client reconnection.
- Schema versioning.

---

## 11. Storage and reports

### 11.1 Local storage

Use SQLite WAL for:

- Run metadata.
- Configuration.
- Aggregate operation statistics.
- One-second time buckets.
- Error groups.
- Redacted samples.
- Worker events.
- SLO evaluations.

High-volume raw request events are not stored by default.

### 11.2 Report bundle

Plain Text

report/

  index.html

  run.json

  summary.json

  operations.csv

  errors.csv

  timeseries.json.zst

  samples.json.zst

  junit.xml

The HTML report must be self-contained and viewable without a running controller.

### 11.3 Retention

Configurable policies:

- Maximum number of runs.
- Maximum age.
- Maximum disk usage.
- Sample retention.
- Baseline protection.
- Automatic compaction.

---

## 12. Distributed execution

### 12.1 Control protocol

Today the [wire contract](../proto/flock.proto) supports telemetry check-ins
and a separate result-only conformance collector. Expected workers run a
local check and submit findings for the active campaign. Workload distribution,
synchronized starts, and rebalancing below are
proposals, not features of `starling bird`.

Controller-agent communication uses gRPC over TLS.

Agent registration includes:

- Agent ID.
- Starling version.
- Supported MCP protocol versions.
- CPU and memory.
- Operating system.
- Transport capabilities.
- Maximum file descriptors.
- Workload artifact hash.

### 12.2 Run synchronization

1. Controller validates the workload.
2. Controller distributes a content-addressed artifact.
3. Agents verify its hash.
4. Controller assigns virtual users or arrival-rate partitions.
5. Agents prepare connections.
6. Controller supplies a future coordinated start time.
7. Agents execute and send aggregate frames.
8. Controller rebalances when configured and safe.
9. Workers drain in-flight operations at run end.

### 12.3 Worker loss

Configurable policies:

- Continue with reduced load.
- Rebalance new users to remaining workers.
- Abort the run.
- Mark the affected interval invalid.

Existing stateful sessions must not be silently recreated on another agent and counted as uninterrupted users.

---

## 13. Security

### 13.1 Safe load generation

- Default controller binding: localhost.
- Require explicit configuration to bind publicly.
- Optional target allowlist.
- Configurable maximum users and operation rate.
- Warning or confirmation for non-development targets.
- Emergency stop always available.
- No automatic retry storm after a broad outage.

### 13.2 Secrets

- Reference environment variables or external secret providers.
- Never persist bearer tokens in run configuration.
- Redact authorization headers, cookies, session IDs, and configured JSON paths.
- Do not expose secrets in frontend payloads.
- Wipe short-lived credentials from agent state after a run.

### 13.3 stdio safety

- Commands use argv arrays, not shell strings.
- Working directory and environment are explicit.
- Child process count and lifetime are bounded.
- Process groups are terminated on abort.
- stdout is reserved for MCP messages; stderr is handled separately.

### 13.4 Distributed security

- Mutual TLS or short-lived signed agent tokens.
- Workload artifact signature/hash verification.
- Run-level authorization.
- Audit trail for starting, stopping, and aborting runs.

Remote MCP authorization should follow the protocol’s authorization model rather than inventing a Starling-specific login exchange.

---

## 14. Reliability and performance requirements

### 14.1 Functional requirements

- Deterministic workload behavior given the same seed and inputs.
- Graceful cancellation and draining.
- No percentile averaging.
- No raw secrets in reports.
- Controller restart must preserve completed run data.
- Worker telemetry loss must be visible.
- MCP session and transport errors must remain distinguishable.

### 14.2 Provisional performance targets

These are engineering acceptance targets, not claims about current implementation:

- Dashboard data no more than two seconds behind the controller.
- Controller aggregation adds less than 5% worker CPU overhead in standard one-second mode.
- Instrumentation adds less than 10% latency overhead against a local no-op MCP benchmark.
- Dashboard remains responsive for a six-hour run with 100 visible time series.
- Agent detects generator saturation before dispatch lag exceeds the configured validity threshold.
- Completed run reports open without the controller.

---

## 15. Testing strategy

### 15.1 Test MCP server

Create `starling-test-server`, supporting stdio and Streamable HTTP, with configurable behavior:

- Fixed and random latency.
- Streaming responses.
- Progress notifications.
- JSON-RPC errors.
- Tool-level errors.
- Invalid responses.
- Dropped connections.
- SSE reconnect and redelivery.
- Session expiration.
- Authentication failures.
- Pagination.
- Long-running operations.
- Sampling and elicitation requests.
- Graceful and abrupt shutdown.

### 15.2 Automated testing

- Unit tests for scheduling and aggregation.
- Property tests for histogram merging and stage transitions.
- Protocol fixture tests.
- Integration tests for both transports.
- Distributed controller-agent tests.
- Network fault and worker-loss tests.
- Six-hour soak test.
- Snapshot tests for reports.
- Playwright dashboard tests.
- Accessibility tests.
- Benchmark suite using Criterion.

### 15.3 Critical invariants

- Sum of per-operation outcomes equals total completed operations.
- In-flight count never becomes negative.
- Merged counters equal the sum of worker counters.
- Percentiles are calculated from merged histograms.
- Paused runs dispatch no new operations.
- Aborted runs terminate subprocess groups.
- Redacted paths never appear in API responses or reports.

---

## 16. Delivery plan

Assuming two Rust engineers and one frontend-capable engineer:

Milestone | Scope | Indicative duration

M0: Protocol spike | `rmcp` adapter, stdio/HTTP instrumentation, no-op test server | 1–2 weeks

M1: Single-node engine | Scenarios, tasks, sessions, load shapes, headless metrics | 2–3 weeks

M2: Frontend | Run controls, live dashboard, graphs, tables, errors | 2–3 weeks

M3: Persistence and CI | Reports, SLO gates, exports, run history | 2 weeks

M4: Distributed mode | Controller-agent protocol, worker dashboard, histogram merging | 3 weeks

M5: Hardening | Fault injection, security, soak tests, packaging, documentation | 2–3 weeks

A credible MVP is approximately **10–13 weeks**, with distributed hardening potentially extending beyond the first public preview.

---

## 17. MVP acceptance criteria

Starling v0.1 is ready when a developer can:

1. Write a Rust scenario with setup, weighted tasks, waits, generated inputs, and assertions.
2. Load test tools and resources over stdio and Streamable HTTP.
3. Run constant-user, ramp, and constant-arrival-rate tests.
4. Start, pause, resume, stop, and abort from the browser.
5. View live throughput, latency percentiles, error rate, users, sessions, tool mix, transport health, and worker health.
6. Drill into a failing tool and inspect redacted samples.
7. Save and reopen a run.
8. Export HTML, CSV, JSON, and JUnit reports.
9. Fail a CI job based on latency or error-rate SLOs.
10. Run the same workload across multiple agents.
11. Receive an explicit warning when the generator, rather than the target, is saturated.
12. Complete a six-hour soak run without unbounded controller or browser memory growth.

## 18. Recommended first implementation slice

Build the first vertical slice around one realistic workflow:

Plain Text

Streamable HTTP target

    → initialize

    → tools/list

    → weighted tools/call loop

    → assertions

    → one-second histograms

    → WebSocket

    → live throughput/latency/error graphs

    → stop

    → persisted report
