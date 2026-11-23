# Quickstart: fly your first flock

A **bird** is one Starling worker process. It can sit idle or run a workload
you give it. It reports to the **controller**, which hosts the dashboard.
Start with two birds on one machine; no credentials or external service needed.

Already have `starling`? [Launch the flock](#launch-the-flock). For Docker
without a host Rust toolchain, see [Linux container](#linux-container).

## Install the CLI

You need Git, [rustup](https://rustup.rs/) (Rust 1.99; the checkout pins
[1.99.1](rust-toolchain.toml)), Node.js 22.12+ with npm, and
a native linker (Xcode Command Line Tools on macOS, `build-essential` on
Debian/Ubuntu). Build the web assets first: Cargo embeds them in the binary.

For the released source rather than current `main`, use
`git clone --branch v0.2.0 https://github.com/monk-eee/starling.git` in
place of the first command below.

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

Once installed, the binary contains the dashboard, fixture and starter YAML.
You do not need this checkout, Node.js, a second repository, or a collector
to run it. `starling init` writes a workload; it is **not** required for a demo.

## Verify the install and read the logs

Run this from your shell on macOS or Linux. A fresh directory checks that
the installed binary does not rely on files in the clone:

```sh
workdir="$(mktemp -d)"
(cd "$workdir" && starling demo --duration 1s --report first-run 2>events.log) &&
  grep -F '"event":"run_finished"' "$workdir/events.log" &&
  test -s "$workdir/first-run/index.html"
```

The CLI prints the fixture address and report path to **stdout**. In the
default configuration, `events.log` captures lifecycle events on **stderr**:
`process_started`, `run_started`, `run_finished`, then `process_finished` on
success. Look for `"status":"completed"`, a positive `requests` count and
`"failed":0` in the `run_finished` row; the exact count varies. Open
`$workdir/first-run/index.html` for the offline report. On a nonzero exit,
read the whole log: stderr can also contain a human-readable diagnostic.

There is no network export in this example. For a collector-enabled install,
see the [OTLP build and runtime settings](developer.md#process-telemetry).

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
   To test the same workload against different MCP revisions, use the
   **MCP protocol revision** dropdown before **Start run**. Afterward select
   **Configure next run**, choose another date, and compare the requested and
   negotiated revisions in each report. This does not reconfigure running birds.

## Join a bird yourself

**Terminal 2**, on the same machine:

```sh
starling bird --id wing-3 --home http://127.0.0.1:8788 --demo
```

Click `wing-3` when it appears. It runs its own fixture and then keeps
reporting. Its requests are sequential with no added pause. If you want a
paced comparison, add `--wait 250ms`; use `--wait 0` to override a paced
workload file. For a telemetry-only check, omit `--demo` and `--wait`: the new bird shows
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
To require a particular revision, set `target.protocol` in the YAML or add
`--protocol 2025-06-18` to the bird command. See the [supported revisions](README.md#test-an-mcp-protocol-revision).
The target must be reachable on the bird's own loopback interface. Load is
per bird: two birds each running `load.users: 2` create four sessions, not two.

Prefer to create the YAML from the CLI? `starling init questions.yaml`
creates a **new** file targeting `127.0.0.1:8081/mcp`. Either start
`starling fixture` in another terminal or change `target.url` to the actual
MCP target before running. **Load YAML** in the browser configures the
controller's next local run; it does not send the file to birds.

## Check a server once or as a fleet

No separate server is needed for either of these commands. Choose new report
directories; Starling will not overwrite a previous result.

```sh
starling check --demo --all-protocols --report one-shot
starling collect --demo-birds 2 --report fleet
```

Open `one-shot/conformance.json` or `fleet/fleet-conformance.json`. The fixture
advertises tools and prompts, not resources, so `catalog.resources` should be
`skip`, **not** `pass`. The fleet command starts two independent processes on
this host; each checks all four revisions and submits evidence over loopback gRPC.
For a real local target, use `starling check --url http://127.0.0.1:8081/mcp --all-protocols`
without a file, or `starling check --config questions.yaml`. Set a matrix in
that YAML or pass `--protocol 2025-11-25,2026-07-28`. With neither specified,
`check` uses `target.protocol`; `auto` checks one negotiated revision, not every revision.

**Across hosts:** provision [flock TLS credentials](developer.md#connect-across-machines)
on the collector and workers. Start the collector first:

```sh
starling collect --listen "${COLLECTOR_PRIVATE_IP:?set the private bind IP}:8790" \
  --expect west,east --deadline-secs 120 --report fleet-run
```

On each worker, set `CAMPAIGN` to the printed **Check campaign** and use the
collector's reachable certificate DNS name, not a wildcard bind address:

```sh
starling check --config questions.yaml --all-protocols --id west \
  --home "https://${COLLECTOR_DNS:?set the collector DNS name}:8790" \
  --campaign "${CAMPAIGN:?copy the campaign ID}" --report west-check
```

Run the same command with `--id east --report east-check` on the second host.
Each worker checks its **own** locally configured target and saves its own
JSON before submitting. `collect` expects all supported revisions by default;
to check fewer, give the collector and every worker the same `--protocol` list.
A missing worker is named at the deadline, and an incomplete campaign cannot
pass. The collector does not send workloads or commands.

## Reports and shutdown

For deployment verification, run the [separate conformance and workload
acceptance gates](README.md#use-conformance-and-performance-as-deployment-gates).
The workload command saves its report even when a performance threshold fails.

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
