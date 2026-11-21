# Starling developer guide

[Run your first flock](QUICKSTART.md) · [Workload fields and limits](README.md#configure-a-workload) · [Design and roadmap](docs/SPEC.md) · [Architecture decisions](docs/adr.d/README.md) · [Contributing and pre-commit](CONTRIBUTING.md)

## Build and run from a checkout

Use the [quickstart prerequisites](QUICKSTART.md#install-the-cli). From the
repository root:

```sh
npm --prefix web ci
npm --prefix web run build
cargo build --locked
cargo run --locked -- demo --web --birds 2
```

Open the printed `Dashboard:` URL. Spawned birds use the same executable as
the controller, so this tests your checkout, not a previous installation on
`PATH`. Rebuild the frontend before rebuilding Rust after UI changes.
[build.rs](build.rs) embeds the single HTML asset through the report module
and compiles the [protobuf schema](proto/flock.proto) with Rust tooling; there
is no runtime Node dependency or system `protoc` requirement. The dashboard
bundles the [bird artwork](web/src/assets/starling-bird.webp) for offline use.
If Cargo reports a missing embedded dashboard, run the two npm commands
above before retrying. To check installation rather than `cargo run`, follow
the [out-of-checkout smoke test](QUICKSTART.md#verify-the-install-and-read-the-logs).

## Follow a bird from CLI to screen

```text
local workload file -> bird -> local MCP target
                        |
                        +-- gRPC CheckIn --> controller roster -- WebSocket --> browser
                        |
                        +-- finished run --> report directory on the bird's host
```

| If you are changing... | Start here |
|---|---|
| Which processes start or stop | [CLI](src/main.rs) binds the fixture and dashboard. [Bird lifecycle](src/bird.rs) spawns demo children, registers independent birds, and drains their local workloads. |
| What the test actually sends | [Config](src/config.rs) validates one selected loopback target and the named requests. The [runner](src/runner.rs) owns the virtual users and timing; the [MCP adapter](src/mcp.rs) sends the method and checks its result. A bird is a process, not a virtual user or worker thread. |
| What a bird sends home | [flock.proto](proto/flock.proto) defines `Telemetry.CheckIn` for the live roster and `CheckCollector.Submit` for a separate, finite conformance campaign. Neither receipt contains a command or target. Optional [OTLP process telemetry](#process-telemetry) uses neither RPC. |
| What the user sees | The [hub](src/flock.rs) keeps the latest heartbeat per ID. The [dashboard](src/dashboard.rs) serves `GET /api/v1/flock` and `/api/v1/flock/live` (WebSocket). The [flock panel](web/src/flock.tsx) renders selectable birds from those snapshots; [feed.ts](web/src/feed.ts) handles browser reconnection. |
| Where results go | The [report writer](src/report.rs) saves the local run. The controller does not collect birds' offline bundles, merge their p95s, or replay the live roster in an export. |

The [protocol selector](README.md#test-an-mcp-protocol-revision) shares the
backend revision list with the dashboard. The configured revision controls
session startup and is stored in `run.json`; `summary.json` also records the
revisions of sessions actually established. A refused pin does not count
as a successfully negotiated revision. The [wire tests](tests/cli/protocol.rs)
check the startup message, subsequent request headers and fallback behavior.
Supervised demo birds inherit the protocol requested at launch; the browser
controls only the controller's local run.

### Extending the conformance suite

The [revision registry](src/config.rs) drives the YAML/CLI parser, dashboard
selector, exact-version matrix, and catalog contract. The [core suite](src/conformance.rs)
uses the same [session adapter](src/mcp.rs) as load tests but opens **one**
session per revision, probes each advertised catalog once, and closes it.
No configured tool arguments, target paths, server error text or catalog
contents enter the report. The reported bird ID must be public.

| Stable rule | Evidence in `mcp.streamable-http.core.v1` |
|---|---|
| `lifecycle.revision` | Successful typed `initialize` or `server/discover`, requested vs negotiated revision and a link to the applicable spec. A counter-offer cannot pass an exact pin. |
| `catalog.tools` | If advertised, an uncached first `tools/list` page matches the expected result type; decoded tool input schemas have `type: object`. An empty list is valid. |
| `catalog.prompts` | If advertised, the server returns a typed first `prompts/list` page. No prompts are expanded. |
| `catalog.resources` | If advertised, the first `resources/list` page decodes and its resource URIs parse as absolute URIs. Their contents are not read. |
| `transport.close` | The SDK closed the client transport. This is an operational observation, not proof of server-side cleanup. |

For `2026-07-28` the catalog rules also check for the complete result marker
and presence of both [caching hints](https://modelcontextprotocol.io/specification/2026-07-28/server/utilities/caching).
Older revisions do not need those fields. This is a **bounded interoperability
profile**, not a raw JSON Schema validator or official certification. It does
not check every wire field (the SDK normalizes some values), every page,
authorization, sampling, notification delivery or the semantics of tool answers.
`skip` means an optional capability wasn't advertised; `not_run` means startup
prevented a probe. A timeout, transport failure or otherwise indeterminate
response is `inconclusive`, not proof that the server broke the specification.

To support a new revision, confirm it in the pinned SDK and published spec,
register it with its catalog requirements, implement any changed lifecycle
or rule, and add positive and counterexample cases to the [wire tests](tests/cli/protocol.rs).
Use stable rule IDs. If evidence semantics or required rules change, version
the suite and update the worker validator and dashboard decoder together;
do not reinterpret an old pass. Load metrics and their report schema are separate.

The [result collector](src/check_fleet.rs) only accepts the current campaign,
listed worker IDs, matching suite/schema, and complete rows for the expected
revisions. Identical resubmissions are idempotent; conflicting ones are
rejected. Workers are trusted observers within the configured TLS trust
domain, not cryptographic proof that a particular target behaved correctly.
The [fleet tests](tests/cli/conformance.rs) check separate worker processes
and missing evidence; the [container test](tests/flock_containers.py) runs
workers and the collector in distinct network namespaces with mutual TLS.

### Heartbeats and identity

- Each bird registers **before** starting an optional workload. Once registered
  it sends cumulative request/failure counts, active sessions, in-flight work,
  state, PID, and optional p95 about once a second. A slow acknowledgement
  does not block the workload task. No raw target path, question, or response
  body is in the heartbeat.
- Identify a bird by its public CLI `--id` (1–32 ASCII letters, digits,
  hyphens, or underscores), **not PID**: two containers can both report PID 1.
  An incarnation distinguishes process lifetimes; a sequence number orders
  heartbeats within one lifetime. A duplicate or old heartbeat neither adds
  requests nor refreshes last-seen time.
- **Completed** describes the workload; **stale** describes the connection.
  The controller uses time since receipt: after 5 seconds without a new
  heartbeat the entry is stale; after 5 minutes it drops out of the roster.
  A restarted process can reclaim its stale ID with fresh counters. A second
  live process using that ID is rejected. The roster retains up to 64 birds;
  `demo --web --birds N` launches 1–8 supervised local demo processes.
- An already-registered bird retries transient telemetry failures, not its MCP
  requests. After roughly 30 seconds without acknowledgement it drains active
  work and exits. Stopping a bird is not the same as stopping the controller's
  local run.

### Inspect the roster

Copy the `Dashboard:` URL into `DASHBOARD`, **not** the gRPC address. From
a terminal in the dashboard's network namespace:

```sh
curl --fail --silent --show-error "$DASHBOARD/api/v1/flock"
```

The JSON contains `home` (the address birds use), `demo_birds_launched`
(a startup count, not a liveness signal), `stale_after_ms`, and `birds`
(latest samples). **Reporting** counts recent heartbeats, not open sockets
or distinct hosts. Compare two snapshots a few seconds apart: for a healthy
completed bird, `sequence` (a decimal string) advances while `requests` stays
fixed, `age_ms` stays low, and `in_flight` is zero. If the browser says **No
recent flock data** or **Telemetry disconnected**, its display is last-known
data: check the controller before diagnosing an individual bird.

## Connect across machines

Keep the browser dashboard on loopback. Only the **separate gRPC listener**
needs to be reachable from the workers, on a private interface. The local
demo uses plaintext on numeric loopback; a non-loopback listener requires
mutual TLS. Start independent birds yourself in this mode rather than using
`--birds`.

Use a dedicated CA and provision PEM files before starting either process:

| Environment variable | On the controller | On each bird |
|---|---|---|
| `STARLING_FLOCK_CA` | Trusted CA certificate file | Trusted CA certificate file |
| `STARLING_FLOCK_CERT` | Server certificate valid for the controller DNS name | This bird's client certificate |
| `STARLING_FLOCK_KEY` | Matching server private key | Matching client private key |

Set all three to **paths**, not PEM text. Keep private keys outside the
repository and mount only the key needed by that process. With those variables
set on the respective hosts, use the following commands. Set
`CONTROLLER_PRIVATE_IP` to the controller's bind address and `CONTROLLER_DNS`
to its reachable name on the server certificate.

```sh
# Controller host
starling demo --web --bird-listen "${CONTROLLER_PRIVATE_IP:?set the private bind IP}:8788"
```

```sh
# Worker host: a self-contained fixture, not traffic to a shared service
starling bird --id wing-1 --home "https://${CONTROLLER_DNS:?set the controller DNS name}:8788" --demo
```

For a real local test server, replace `--demo` with `--config questions.yaml`
on the worker. `--home` addresses the controller; `target.url` in that file
addresses the MCP service **from the worker's network namespace**. An HTTPS
telemetry connection does not make remote workload targets valid. If binding
to a wildcard address inside a container, advertise the reachable DNS name
to birds, not the wildcard printed as the listener address.

## Process telemetry

The [structured logger](src/telemetry.rs) and [OTLP exporter](src/telemetry/otlp.rs)
build entirely from this checkout and public Rust crates. They observe the
local process. They neither send work to birds nor replace the flock heartbeat.
The runner, controller, birds, check workers, result collector and standalone fixture emit process lifecycle events;
`init` and `validate` do not initialize exporters.

| Setting | Behavior |
|---|---|
| `STARLING_TELEMETRY` | `stderr` (default), `none`, `otlp`, or `stderr,otlp` (either order). Setting a collector origin alone does not enable export. Selecting OTLP in a binary built without the `otlp` feature fails at startup. |
| `STARLING_LOG_LEVEL` | Minimum `info` (default), `warn`, `error`, or `off`. This filters event logs, not spans, metrics or the CLI's exit diagnostics. |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Required when OTLP is selected. Use an HTTPS origin or numeric loopback HTTP for local testing. No user info, path prefix, query, or fragment. All three signals use this origin. Standard OTLP headers may carry collector credentials; keep those in the operator environment, not workload YAML. |

Build the web assets first, then `cargo install --path . --locked --features otlp`.
For an image with the exporter, use
`docker build --build-arg STARLING_FEATURES=otlp -t starling-otlp .`.
Inside a container, loopback means **that container**. Supervised demo birds
inherit the controller environment; configure independent birds where they run.

- **Logs:** one JSON object per line on stderr with `timestampMs`, `service`,
  `level`, `event` and `role`. Birds include their public `birdId`. Run completion
  includes `status`, `requests` and `failed`; when tracing is enabled the run
  logs also include `traceId` and `spanId`. Output addresses on stdout and
  human-readable CLI errors on stderr are separate from these records.
- **Spans:** one `starling.run` span per started workload. There are no
  per-request spans and no trace headers are injected into the target.
- **Metrics:** `starling.workload.requests` is a monotonic counter.
  `starling.workload.duration` is a histogram of measured request durations
  **in milliseconds**. Both use `mcp.method.name` and `starling.outcome`;
  sum non-`success` outcomes for failures. Setup/discovery, think time and
  export are not measured requests. Each process has its own `service.instance.id`;
  combine histogram buckets for fleet percentiles, not the heartbeat p95s.
- **Delivery:** collecting a request measurement does not write to stderr or
  wait for a telemetry network request.
  The stderr queue is bounded, logs and spans are batched, and metrics export
  periodically and on graceful shutdown. The HTTP client does not use ambient
  proxies, follow redirects, or retry requests. Export is best effort, not a
  delivery receipt. A detected shutdown/write error produces a CLI diagnostic
  and nonzero exit; it does not rewrite an already-saved workload report.

Run `cargo test --locked --all-features --test cli telemetry::` to exercise a
real CLI against a disposable protobuf receiver, without cloud credentials.
The [data-handling proposal](docs/adr.d/0005-process-telemetry.md) lists the
fields that must stay out of exported telemetry.

### How logging starts and stops

1. The [CLI entry point](src/main.rs) reads the subcommand and assigns a
   role: `runner` for headless work, `controller` for a dashboard, `bird` for
   a worker, `fixture` for a standalone test server, or `check` / `collector`
   for conformance commands. `init` and `validate`
   skip the log writer and exporters, even if collector settings are present.
2. Before starting work, [logging setup](src/telemetry.rs) resolves the
   destinations and level. The default opens a bounded stderr queue; only an
   explicit OTLP selection constructs the three [SDK providers](src/telemetry/otlp.rs).
   `process_started` means the process is starting work, **not** that a
   collector received anything.
3. The [runner](src/runner.rs) starts a span when a workload actually starts.
   `run_finished` records its outcome and counts after the report write is
   attempted. A report-write failure records `failed`, not `completed`.
   A bird may keep sending heartbeats after its workload ends.
4. On graceful exit the process emits `process_finished` or `process_failed`,
   then drains its writer and shuts down its providers. A final export error
   can still make the CLI exit nonzero after a successful local run. Check
   the **exit status**, not just the last JSON event.

## Test changes

From the clone, after building the web assets:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo build --locked --release
```

For a dashboard change, build the **debug** executable too (`cargo build --locked`),
then run:

```sh
cd web
npx playwright install chromium
npm run test:e2e
```

The [browser tests](web/tests/flow.spec.mjs) launch real processes on free
ports, select a bird, disconnect and rejoin it, load a YAML scenario, and
open a report offline. Use `STARLING_BROWSER_CHANNEL=chrome` if testing
with an installed Chrome instead.

### Measure test coverage

The [CLI workflow](.github/workflows/cli.yml) measures two different things on
Linux. Open a [workflow run](https://github.com/monk-eee/starling/actions/workflows/cli.yml)
for the covered/total counts per file. Download `coverage-rust` or
`coverage-dashboard` for LCOV with individual missed lines. The dashboard
artifact also has a browsable `html/index.html`. There is no combined score.

| Check | What runs | Minimum |
|---|---|---|
| Rust | Unit and CLI tests, including their instrumented child processes and the two Unix shutdown tests | 75% of executable lines |
| Dashboard | Chromium drives the real CLI and embedded dashboard, including reload and opening an exported report offline | 90% of executable lines and 80% of branches |

From the clone on macOS or Linux, with Python 3, Node 22 and the pinned Rust
toolchain available:

```sh
npm --prefix web ci
npm --prefix web run build
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.8.7 --locked
python3 tests/coverage.py rust

(cd web && npx playwright install chromium)  # On a fresh Linux runner use install --with-deps chromium.
python3 tests/coverage.py dashboard
```

The commands use the same [collector and thresholds](tests/coverage.py) as
CI. Results go in `target/coverage/{rust,dashboard}/`. Rust counts each
executable source line once, not once per overlapping function. A child
that is forcibly killed doesn't get to flush its profile. Keep Rust
unit tests in `tests.rs` under their module so test assertions don't inflate
the number. Dashboard instrumentation runs on the authored TS/TSX before
bundling, in a separate build; it doesn't replace your normal CLI or web
assets. Unvisited modules still count. A missing source file, missing browser
capture, failed test, or below-minimum result fails the check.

These are whole-suite floors, not per-file or changed-line guarantees. Rust
branch coverage isn't measured. Generated protobuf, dependencies, tests,
CSS and build scripts aren't in the denominators. The separate mutual-TLS
container check isn't folded into either number. Coverage shows what ran,
not whether every outcome was asserted or the service survived a long soak.

### Verify two Linux containers

With Docker, Python 3, and OpenSSL on the host, run the same integration
check as [CI](.github/workflows/cli.yml) from the repository root:

```sh
docker build --build-arg STARLING_FEATURES=otlp -t starling .
docker pull node:22-bookworm-slim
python3 tests/flock_containers.py starling node:22-bookworm-slim
```

The [test](tests/flock_containers.py) creates temporary certificates and an
internal Docker network. It requires two authenticated birds with successful
requests, rejects an untrusted client certificate, stops one bird, waits for
**stale**, then restarts it and checks that there are two live IDs again.
It cleans up its containers, network and test keys. This verifies telemetry
between processes; it is not a benchmark of a production MCP target.

## Known gaps

- **A reporting flock is not a distributed scheduler.** Start work on each
  bird; there is no controller-side dispatch or global concurrency budget.
  [Telemetry boundary](docs/adr.d/0002-telemetry-is-not-dispatch.md)
- **The flock dashboard has no fleet p95.** Its gRPC heartbeat contains a
  bird's p95, not its histogram. A separately configured collector can
  aggregate OTLP histograms; that result is not fed back into the dashboard.
  [Measurement boundary](docs/adr.d/0003-percentiles-need-distributions.md)
- **Duration support is not soak qualification.** The CLI accepts runs up
  to seven days. The [bounded-timeline regression](src/metrics.rs) simulates
  elapsed time; it does not demonstrate a seven-day run on a real target.
- **Reports are written on drain, one file at a time.** A storage failure can
  leave a summary on disk without a complete HTML report. Check the CLI exit
  status; the presence of the directory alone does not mean export succeeded.
  There is no mid-run checkpoint. See the [report writer](src/report.rs).
