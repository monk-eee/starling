import { useState } from "react";
import { createRoot } from "react-dom/client";
import { api, terminal, useFeed } from "./feed";
import { Chart, errors, percentiles, throughput } from "./chart";
import { LoadForm, SettingsSummary, durationLabel, type LoadSettings } from "./load-settings";
import starlingBird from "./assets/starling-bird.webp?inline";
import "./style.css";

const number = (n: number | null, digits = 1) => n === null ? "\u2014" : n.toLocaleString("en", { maximumFractionDigits: digits });
const ms = (n: number | null) => n === null ? "\u2014" : `${number(n, 2)} ms`;
const percentage = (failed: number, count: number) => count ? `${number(100 * failed / count, 2)}%` : "\u2014";

function Dashboard() {
  const { current, offline, connection, error } = useFeed();
  const [pending, setPending] = useState(false);
  const [actionError, setActionError] = useState("");
  async function control(action: "start" | "stop" | "next", options?: Partial<LoadSettings>) {
    setPending(true);
    setActionError("");
    try {
      const response = await fetch(`${api}/${action}`, {
        method: "POST", headers: { "X-Starling-Control": "1", "Content-Type": "application/json" },
        ...(options && { body: JSON.stringify(options) }),
      });
      if (!response.ok) {
        const detail = response.status === 422 ? await response.text() : `HTTP ${response.status}`;
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
  const disabled = pending || connection !== "Connected";
  return <main>
    <nav>
      <strong><img src={starlingBird} alt="" width={60} height={40} />Starling</strong>
      <span> / &nbsp; MCP performance</span><small>{connection}</small>
    </nav>
    {(error || actionError) && <p className="alert" role="alert">{error || actionError}</p>}
    {!current ? <h1>Connecting to the run...</h1> : <>
      <header className="intro">
        <div><p className="eyebrow">{offline ? "Saved report" : "Local workload"}</p><h1>{current.run.name}</h1>
          <p className="endpoint">{current.run.target_origin} &middot; Streamable HTTP
            {current.protocols.length > 0 && ` / MCP ${current.protocols.join(", ")}`}</p></div>
        <div className="actions">
          <span className={`badge ${current.status}`} aria-label="Run status" role="status">{current.status}</span>
          {!offline && current.status === "ready" && <button disabled={disabled}
            type="submit" form="load-settings">Start run</button>}
          {!offline && (current.status === "running" || current.status === "stopping") && <button
            disabled={disabled || current.status === "stopping"} onClick={() => void control("stop")}>
            {current.status === "stopping" ? "Draining..." : "Stop run"}</button>}
          {!offline && terminal(current.status) && <>
            <a className="button" href={`${api}/export`} download>Export HTML</a>
            <button disabled={disabled} onClick={() => void control("next")}>Configure next run</button>
          </>}
        </div>
      </header>
      {current.reason && <p className="alert" role="alert">{current.reason}</p>}
      {settings && (!offline && current.status === "ready"
        ? <LoadForm initial={settings} maxDurationMs={current.run.max_duration_ms ?? 900000}
            disabled={disabled} onStart={options => void control("start", options)} />
        : <SettingsSummary settings={settings} />)}
      <section className="kpis" aria-label="Run metrics">
        {[
          ["Tool calls", number(current.calls.count, 0), `${number(current.calls.failed, 0)} failed`],
          ["Throughput", last ? `${number(last.calls * 1000 / last.span_ms)}/s` : "\u2014",
            current.open_interval ? "Current partial interval" : "Latest interval"],
          ["p95 latency", ms(current.calls.p95_ms), "All completed tool calls"],
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
      <div className="section-title"><h2>Operations</h2><small>Setup is measured separately from tool calls</small></div>
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
