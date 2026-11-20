# Quickstart: fly your first flock

A **bird** is one Starling worker process. It can sit idle or run a workload
you give it. It reports to the **controller**, which hosts the dashboard.
Start with two birds on one machine; no credentials or external service needed.

Already have `starling`? [Launch the flock](#launch-the-flock). For Docker
without a host Rust toolchain, see [Linux container](#linux-container).

## Install the CLI

You need Git, [rustup](https://rustup.rs/), Node.js 22.12+ with npm, and
a native linker (Xcode Command Line Tools on macOS, `build-essential` on
Debian/Ubuntu). Build the web assets first: Cargo embeds them in the binary.

```sh
git clone https://github.com/monk-eee/starling.git
cd starling
npm --prefix web ci
npm --prefix web run build
cargo install --path . --locked
starling bird --help
```

If `starling` is not found after installing, add Cargo's bin directory to
your shell's `PATH` (usually `export PATH="$HOME/.cargo/bin:$PATH"`).

## Launch the flock

**Terminal 1**, in any writable directory:

```sh
starling demo --web --birds 2 --bird-listen 127.0.0.1:8788
```

Leave it running. The CLI prints three addresses with different jobs:

| Output | What to do with it |
|---|---|
| `Dashboard:` | Open this URL in your browser. The demo chooses a free port. |
| `Bird telemetry:` | Pass this URL to a bird's `--home`. In this walkthrough it is `http://127.0.0.1:8788`. |
| `Demo fixture:` | This is an MCP **target**, not a dashboard or telemetry address. |

If port 8788 is busy, omit `--bird-listen 127.0.0.1:8788`. Use the printed
telemetry address in the commands below, or copy it from **Connect a bird**.

1. In **The flock**, click `bird-01`, then `bird-02`. Each has its own PID,
   heartbeat number, request count, failures, and p95 latency. **Last heard**
   measures time since telemetry arrived. A **p95** of 20 ms means 95% of that
   bird's completed workload requests took about 20 ms or less.
2. No click is needed to start the birds. Each runs one user against its own
   built-in MCP fixture for 15 seconds. Afterward, expect **2 reporting,
   0 running, 0 stale**, nonzero request counts, and `completed` on the birds.
   Their heartbeat numbers keep rising; their request counts stop. These are
   two local processes, not two remote machines.
3. **Start run** below the flock is optional. It starts the *controller's
   local workload*. Its charts and **Export HTML** are not the sum of the birds.

## Join a bird yourself

**Terminal 2**, on the same machine:

```sh
starling bird --id wing-3 --home http://127.0.0.1:8788 --demo
```

Click `wing-3` when it appears. It runs its own fixture and then keeps
reporting. For a telemetry-only check, omit `--demo`: the new bird shows
`idle` and sends no MCP requests.

Press Ctrl+C **in terminal 2 only**. About five seconds after its last
heartbeat, `wing-3` fades to **stale**; the other two stay connected. Once
it is stale, rerun the same command. The new process takes over the same
tile with fresh counters. A new process cannot take over an ID that is still live.

## Run your questions on a bird

Keep terminal 1 running. On its dashboard, choose **Download example YAML**.
Save it as `questions.yaml` in terminal 2's working directory. The demo
download already points to terminal 1's fixture, so no additional server is
needed. Change the question in `tasks[].arguments.text` for an `echo` case,
or set the target, methods, arguments, and assertions for a different local
test service you control. See the [annotated example](examples/local.yaml).

```sh
starling validate --config questions.yaml
starling bird --id wing-yaml --home http://127.0.0.1:8788 --config questions.yaml
```

Validation sends no requests. The bird starts its workload **after it
registers**, without pressing **Start run**. Select `wing-yaml` to watch its
counts. To use a named target from `target.endpoints`, add `--endpoint NAME`.
The target must be reachable on the bird's own loopback interface. Load is
per bird: two birds each running `load.users: 2` create four sessions, not two.

Prefer to create the YAML from the CLI? `starling init questions.yaml`
creates a **new** file targeting `127.0.0.1:8081/mcp`. Either start
`starling fixture` in another terminal or change `target.url` to the actual
MCP target before running. **Load YAML** in the browser configures the
controller's next local run; it does not send the file to birds.

## Reports and shutdown

| Run | Where to open its offline report |
|---|---|
| `demo --web --birds 2` child | `out/run-…/birds/bird-01/index.html` (and `bird-02/`) under terminal 1's working directory. The parent is the controller's **initial** report directory. |
| Manually launched bird with `--config` or `--demo` | The `index.html` path printed by that bird, on its host. Add `--report PATH` for a **new** directory; omit it for a fresh `out/run-…/` each time. |
| Controller's local run | Its printed report path or **Export HTML** in the dashboard. Offline HTML shows a saved run, not a live flock. |

Reports are written when work finishes or drains. A bird that finished
successfully can keep checking in without issuing any more requests. Stop
manually launched birds with Ctrl+C in their terminals. Ctrl+C in terminal 1
shuts down the controller, its fixture, and the demo birds it launched.

## Linux container

For a headless check from the clone, using Docker on Linux or macOS:

```sh
docker build -t starling .
mkdir -p out
docker run --rm --network none --user "$(id -u):$(id -g)" \
  -v "$PWD/out:/work/out" starling demo --duration 5s
```

Open the printed report under the host's `out/`. For a long run, use
`docker stop --timeout 45 CONTAINER` (substitute its name or ID) to allow time
for draining and saving rather than forcing a kill. For birds in **separate
containers**, use the [two-container check](developer.md#verify-two-linux-containers).
Loopback in one container is not the host or a sibling container.
