import { useState } from "react";
import { createRoot } from "react-dom/client";
import { api, preparedRun, terminal, useFeed, type Frame } from "./feed";
import { Chart, errors, percentiles, throughput } from "./chart";
import { Conformance, checkReport, type CheckReport } from "./conformance";
import { Flock } from "./flock";
import { LoadForm, SettingsSummary, durationLabel, type StartOptions } from "./load-settings";
import starlingBird from "./assets/starling-bird.webp?inline";
import "./style.css";

const number = (n: number | null, digits = 1) => n === null ? "\u2014" : n.toLocaleString("en", { maximumFractionDigits: digits });
const ms = (n: number | null) => n === null ? "\u2014" : `${number(n, 2)} ms`;
const percentage = (failed: number, count: number) => count ? `${number(100 * failed / count, 2)}%` : "\u2014";

function ExportReport() {
  const [file, setFile] = useState("index.html");
  return <>
    <label>Download file <select value={file} onChange={e => setFile(e.currentTarget.value)}>
      <option value="index.html">HTML dashboard</option>
      <option value="summary.json">JSON summary</option>
      <option value="summary.yaml">YAML summary</option>
      <option value="run.json">JSON run settings</option>
      <option value="timeseries.json">JSON timeline</option>
      <option value="operations.csv">CSV operations</option>
      <option value="errors.csv">CSV errors</option>
      <option value="junit.xml">JUnit XML</option>
    </select></label>
    <a className="button" href={`${api}/export?file=${file}`} download>
      {file === "index.html" ? "Export HTML" : `Export ${file}`}
    </a>
  </>;
}

function CaseAssertions({ rows, once }: { rows: NonNullable<Frame["assertions"]>; once: boolean }) {
  const passed = rows.filter(r => r.status === "passed").length;
  const verdict = {pending:"Pending", passed:"Passed", failed:"Failed", not_run:"Not run"};
  return <section id="case-assertions" className="panel check-result" aria-label="Case assertions">
    <h2>Case assertions: {passed}/{rows.length} passed</h2>
    <p>{once ? "Each case runs once. Unexecuted cases cannot pass." :
      "Cases are sampled by weight. A case with any failed call stays failed."}</p>
    <div className="table-wrap case-rows"><table><thead><tr>
      <th>#</th><th>Case</th><th>Expected</th><th>Observed</th><th>Verdict</th>
    </tr></thead><tbody>{rows.map((r,i) => <tr key={r.operation}>
      <th scope="row">{i+1}</th><td>{r.title}</td>
      <td>{r.expected}{r.first !== undefined && <small>All conditions on one of the first {r.first} results</small>}</td>
      <td>{r.passed+r.failed === 0 ? "Not executed" : `${r.passed} passed, ${r.failed} failed`}
        {r.failure_kinds && <small>{r.failure_kinds.map(k => k === "assertion" ? "Result did not match" : k.replaceAll("_"," ")).join(", ")}</small>}
        {r.last_match_rank !== undefined && <small>Last match: #{r.last_match_rank}</small>}</td>
      <td className={`case-${r.status}`}>{verdict[r.status]}</td>
    </tr>)}</tbody></table></div>
  </section>;
}

function Dashboard() {
  const { current, offline, connection, error } = useFeed();
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState("");
  const [yamlError, setYamlError] = useState("");
  const [loadedYaml, setLoadedYaml] = useState<{ generation: number; source: string } | null>(null);
  const [pastedYaml, setPastedYaml] = useState("");
  const [checked, setChecked] = useState<{ generation: number | undefined; report: CheckReport } | null>(null);
  const [checking, setChecking] = useState(false);
  async function check(options: { endpoint?: string; protocol?: string; all_protocols: boolean }) {
    setChecking(true); setActionError(""); setChecked(null);
    try {
      const response = await fetch(`${api}/check`, { method: "POST",
        headers: { "X-Starling-Control": "1", "Content-Type": "application/json" }, body: JSON.stringify(options) });
      if (!response.ok) throw new Error(`Could not check conformance: HTTP ${response.status}.`);
      setChecked({ generation: current?.generation, report: checkReport.parse(await response.json()) });
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : "The controller is unreachable.");
    } finally { setChecking(false); }
  }
  async function loadYaml(source: File | string) {
    setPending(true);
    setActionError("");
    setYamlError("");
    try {
      const size = typeof source === "string" ? new Blob([source]).size : source.size;
      if (size > 65536) throw new Error("Workload files must be 64 KiB or smaller.");
      let text: string;
      if (typeof source === "string") {
        text = source;
      } else {
        try { text = new TextDecoder("utf-8", { fatal: true }).decode(await source.arrayBuffer()); }
        catch { throw new Error("Could not read the file as UTF-8. Save it as UTF-8 or paste the YAML instead."); }
      }
      const response = await fetch(`${api}/workload`, {
        method: "POST", headers: { "X-Starling-Control": "1", "Content-Type": "application/yaml" }, body: text,
      });
      if (!response.ok) {
        const body = await response.text();
        const detail = response.status === 422 ? body :
          response.status === 409 ? "A run is already in progress." : `HTTP ${response.status}`;
        throw new Error(detail);
      }
      const receipt = preparedRun.parse(await response.json());
      setLoadedYaml({ generation: receipt.generation, source: typeof source === "string" ? "pasted YAML" : source.name });
      if (typeof source === "string") setPastedYaml("");
    } catch (cause) {
      setYamlError(`Could not load workload: ${cause instanceof Error ? cause.message : "The controller is unreachable."}`);
    } finally { setPending(false); }
  }
  async function control(action: "start" | "stop" | "next", options?: StartOptions) {
    setPending(true);
    setActionError("");
    try {
      const response = await fetch(`${api}/${action}`, {
        method: "POST", headers: { "X-Starling-Control": "1", "Content-Type": "application/json" },
        ...(options && { body: JSON.stringify(options) }),
      });
      const body = await response.text();
      if (!response.ok) {
        const detail = response.status === 422 ? body : `HTTP ${response.status}`;
        throw new Error(`Could not ${action === "next" ? "prepare the next run" : `${action} this run`}: ${detail}.`);
      }
    } catch (cause) {
      setActionError(cause instanceof Error ? cause.message : "The controller is unreachable.");
    } finally { setPending(false); }
  }
  const points = current?.open_interval ? [...current.series, current.open_interval] : current?.series ?? [];
  const last = points.at(-1);
  const run = current?.run;
  const settings = run && run.threads !== undefined && run.wait_ms !== undefined && run.timeout_ms !== undefined
    ? { users: run.users, threads: run.threads, duration_ms: run.duration_ms, wait_ms: run.wait_ms, timeout_ms: run.timeout_ms }
    : null;
  const disabled = pending || checking || connection !== "Connected";
  return <main>
    <nav>
      <strong><img src={starlingBird} alt="" width={60} height={40} />Starling</strong>
      <span> / &nbsp; MCP performance</span><small>{connection}</small>
    </nav>
    {(error || actionError) && <p className="alert" role="alert">{error || actionError}</p>}
    {!offline && <Flock />}
    {!current ? <h1>Connecting to the run...</h1> : <>
      <header className="intro">
        <div><p className="eyebrow">{offline ? "Saved report" : "Local workload"}</p><h1>{current.run.name}</h1>
          <p className="endpoint">{current.run.endpoint && current.run.endpoint !== "default" && `${current.run.endpoint}: `}{current.run.target_origin} &middot; Streamable HTTP
            {current.status !== "ready" && current.run.protocol && <span> &middot; Requested MCP: {current.run.protocol}</span>}
            {current.protocols.length > 0 ? <span> &middot; Negotiated MCP: {current.protocols.join(", ")}</span>
              : terminal(current.status) && <span> &middot; No negotiated revision recorded</span>}</p></div>
        <div className="actions">
          <span className={`badge ${current.status}`} aria-label="Run status" role="status">{current.status}</span>
          {!offline && current.status === "ready" && <button disabled={disabled}
            type="submit" form="load-settings">Start run</button>}
          {!offline && (current.status === "running" || current.status === "stopping") && <button
            disabled={disabled || current.status === "stopping"} onClick={() => void control("stop")}>
            {current.status === "stopping" ? "Draining..." : "Stop run"}</button>}
          {!offline && terminal(current.status) && <>
            <ExportReport />
            <button disabled={disabled} onClick={() => void control("next")}>Configure next run</button>
          </>}
        </div>
      </header>
      {current.reason && <p className="alert" role="alert">{current.reason}</p>}
      {current.assertions && current.assertions.length > 0 && <CaseAssertions rows={current.assertions} once={run?.mode === "once"} />}
      {!offline && (current.status === "ready" || terminal(current.status)) &&
        <section className="panel workload-file" aria-label="YAML workload">
          <div><h2>YAML workload</h2><a href="/api/v1/workload/example" download="starling.yaml">Download example YAML</a></div>
          <label>Load YAML <input type="file" accept=".yaml,.yml" disabled={disabled}
            onClick={e => { e.currentTarget.value = ""; }}
            onChange={e => { const file = e.currentTarget.files?.[0]; if (file) void loadYaml(file); }} /></label>
          <details className="paste-yaml"><summary>Paste YAML instead</summary>
            <label>YAML text<textarea rows={6} spellCheck={false} disabled={disabled} value={pastedYaml}
              onChange={e => setPastedYaml(e.currentTarget.value)} /></label>
            <button type="button" disabled={disabled || !pastedYaml.trim()} onClick={() => void loadYaml(pastedYaml)}>Load pasted YAML</button>
          </details>
          {yamlError && <p className="alert" role="alert">{yamlError}</p>}
          <p role="status" aria-label="Loaded workload">
            {loadedYaml && loadedYaml.generation === current.generation
              ? <strong>Loaded {loadedYaml.source}: {current.run.name}.</strong>
              : <strong>Current workload: {current.run.name}.</strong>}
            {current.status === "ready" && " Ready to start. No workload requests sent."}
          </p>
          <small>The download is the fixture starter. Load your own YAML to change the endpoint and questions.</small>
          {current.run.requests && <p>Configured requests: {current.run.mode === "once"
            ? `${current.run.requests.length} cases, each scheduled once` : current.run.requests.join(", ")}</p>}
        </section>}
      {settings && (!offline && current.status === "ready"
        ? <LoadForm key={current.generation} initial={settings} maxDurationMs={current.run.max_duration_ms ?? 900000}
            disabled={disabled} endpoint={current.run.endpoint ?? "default"} endpoints={current.run.endpoint_names ?? []}
            protocol={current.run.protocol} revisions={current.run.protocol_revisions}
            initialFormats={current.run.report_formats}
            onStart={options => void control("start", options)} onCheck={options => void check(options)} />
        : <SettingsSummary settings={settings} />)}
      {checking && <p role="status">Checking MCP conformance...</p>}
      {!offline && checked && checked.generation === current.generation && <Conformance report={checked.report} />}
      {run?.assert && <section className="panel check-result" aria-label="Performance gate">
        <h2>Performance gate{current.acceptance && `: ${current.acceptance.status}`}</h2>
        {current.acceptance ? <div className="table-wrap"><table>
          <thead><tr><th>Metric</th><th>Required</th><th>Observed</th><th>Result</th></tr></thead>
          <tbody>{current.acceptance.checks.map(c => <tr key={c.metric}>
            <th scope="row">{c.metric}</th><td>{c.expected}</td>
            <td>{c.observed === null ? "No samples" : `${c.observed} ${c.unit}`}</td>
            <td>{c.passed === null ? "Not evaluated" : c.passed ? "Pass" : "Fail"}</td>
          </tr>)}</tbody>
        </table></div> : <p>Checked after the workload finishes: {Object.entries(run.assert)
          .map(([metric, value]) => `${metric} ${metric === "min_requests" ? ">= " : ""}${value}`).join("; ")}</p>}
      </section>}
      <section className="kpis" aria-label="Run metrics">
        {[
          ["Workload requests", number(current.calls.count, 0), `${number(current.calls.failed, 0)} failed`],
          ["Throughput", last ? `${number(last.calls * 1000 / last.span_ms)}/s` : "\u2014",
            current.open_interval ? "Current partial interval" : "Latest interval"],
          ["p95 latency", ms(current.calls.p95_ms), "All completed workload requests"],
          ["Error rate", percentage(current.calls.failed, current.calls.count), "Includes assertion failures"],
          ["Active sessions", `${current.sessions}`, `${current.in_flight} operations in flight`],
        ].map(([label, value, note]) => <div key={label} className="kpi"><h2>{label}</h2><strong>{value}</strong><small>{note}</small></div>)}
      </section>
      <div className="section-title"><h2>Timeline</h2><small>{number(current.elapsed_ms / 1000)}s elapsed
        {" / "}{durationLabel(current.run.duration_ms)} planned &middot; {current.run.users} configured users
        {" "}&middot; {current.run.bucket_secs ?? 1}s intervals</small></div>
      <div className="graphs">
        <Chart title="Throughput" unit="calls/s" points={points} lines={throughput} />
        <Chart title="Latency" unit="ms" points={points} lines={percentiles} />
        <Chart title="Error rate" unit="%" points={points} lines={errors} />
      </div>
      <div className="section-title"><h2>Operations</h2><small>Setup is measured separately from scheduled requests</small></div>
      <section className="panel table-wrap"><table><thead><tr>
        {["Operation", "Count", "Failed", "p50", "p95", "p99", "Failure categories"].map(s => <th key={s}>{s}</th>)}
      </tr></thead><tbody>
        {Object.entries(current.operations).map(([name, s]) => <tr key={name}>
          <th scope="row">{name}</th><td>{s.count}</td><td>{s.failed}</td><td>{ms(s.p50_ms)}</td>
          <td>{ms(s.p95_ms)}</td><td>{ms(s.p99_ms)}</td>
          <td>{Object.entries(s.outcomes).filter(([k]) => k !== "success").map(([k, v]) => `${k}: ${v}`).join(", ") || "\u2014"}</td>
        </tr>)}
        {!Object.keys(current.operations).length && <tr><td colSpan={7}>No operations recorded yet.</td></tr>}
      </tbody></table></section>
    </>}
  </main>;
}

createRoot(document.getElementById("root")!).render(<Dashboard />);
