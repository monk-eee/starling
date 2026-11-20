# Starling developer guide

[Run your first flock](QUICKSTART.md) · [Workload fields and limits](README.md#configure-a-workload) · [Design and roadmap](docs/SPEC.md)

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
| What crosses the network | [flock.proto](proto/flock.proto) defines one unary `CheckIn(Heartbeat) -> Receipt` RPC. The receipt is empty: there is no remote start, stop, target assignment, or YAML upload to a bird. |
| What the user sees | The [hub](src/flock.rs) keeps the latest heartbeat per ID. The [dashboard](src/dashboard.rs) serves `GET /api/v1/flock` and `/api/v1/flock/live` (WebSocket). The [flock panel](web/src/flock.tsx) renders selectable birds from those snapshots; [feed.ts](web/src/feed.ts) handles browser reconnection. |
| Where results go | The [report writer](src/report.rs) saves the local run. The controller does not collect birds' offline bundles, merge their p95s, or replay the live roster in an export. |

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

### Verify two Linux containers

With Docker, Python 3, and OpenSSL on the host, run the same integration
check as [CI](.github/workflows/cli.yml) from the repository root:

```sh
docker build -t starling .
docker pull node:22-bookworm-slim
python3 tests/flock_containers.py starling node:22-bookworm-slim
```

The [test](tests/flock_containers.py) creates temporary certificates and an
internal Docker network. It requires two authenticated birds with successful
requests, rejects an untrusted client certificate, stops one bird, waits for
**stale**, then restarts it and checks that there are two live IDs again.
It cleans up its containers, network and test keys. This verifies telemetry
between processes; it is not a benchmark of a production MCP target.
