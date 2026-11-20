import { useEffect, useState } from "react";
import { z } from "zod";
import { subscribeLive } from "./feed";
import bird from "./assets/starling-bird.webp?inline";

const counter = z.number().int().nonnegative();
const rosterSchema = z.object({
  home: z.string(),
  birds: z.array(z.object({
    id: z.string(), pid: counter, state: z.enum(["idle", "ready", "running", "stopping", "completed", "stopped", "failed"]),
    age_ms: counter, stale: z.boolean(), requests: counter, failed: counter, sessions: counter, in_flight: counter,
    p95_ms: z.number().nonnegative().nullable(),
  })),
});

export function Flock() {
  const [roster, setRoster] = useState<z.infer<typeof rosterSchema> | null>(null);
  const [connection, setConnection] = useState("Connecting to flock");
  const [invalid, setInvalid] = useState(false);
  const [selected, setSelected] = useState<string>();
  useEffect(() => subscribeLive("/api/v1/flock/live", value => {
    setRoster(rosterSchema.parse(value)); setConnection("Live"); setInvalid(false);
  }, () => setConnection("Telemetry disconnected"), () => setInvalid(true)), []);
  const birds = roster?.birds ?? [];
  const current = birds.find(b => b.id === selected) ?? birds[0];
  const stale = birds.filter(b => b.stale).length;
  const running = birds.filter(b => !b.stale && b.state === "running").length;
  return <section className="panel flock" aria-label="Live flock">
    <header><div><p className="eyebrow">Worker telemetry</p><h2>The flock</h2></div>
      <p className="flock-count">{connection === "Live" ? `${birds.length - stale} connected · ${running} running · ${stale} stale` : connection}</p>
    </header>
    {invalid && <p role="alert">The controller sent an incompatible flock snapshot.</p>}
    {roster && (birds.length ? <>
      <div className="flock-sky" aria-label="Reporting birds">
        {birds.map(b => <button key={b.id} className="flock-bird" data-state={connection !== "Live" ? "unknown" : b.stale ? "stale" : b.state}
          aria-label={`Bird ${b.id}`} aria-pressed={current?.id === b.id} onClick={() => setSelected(b.id)}>
          <img src={bird} alt="" width={60} height={40} /><strong>{b.id}</strong>
          <small>{connection !== "Live" ? "last known" : b.stale ? "stale" : b.state}</small>
        </button>)}
      </div>
      {current && <div className="flock-detail" aria-label="Selected bird">
        <strong>{current.id}</strong><span>PID {current.pid}</span>
        <span>{connection === "Live" ? `Last heard ${(current.age_ms / 1000).toFixed(1)}s ago` : "Last known sample"}</span>
        <span>{current.requests.toLocaleString("en")} requests · {current.failed} failed</span>
        <span>{current.sessions} sessions · {current.in_flight} in flight</span>
        <span>p95 {current.p95_ms === null ? "—" : `${current.p95_ms.toFixed(2)} ms`}</span>
      </div>}
    </> : <p className="flock-empty">No birds have checked in. Start a bird to see its real heartbeat here.</p>)}
    {roster && <details><summary>Connect a bird</summary>
      <p>Run a reporting process with <code>starling bird --id wing-1 --home {roster.home}</code>.
        Add <code>--config your-workload.yaml</code> on that worker to run a local scenario. Check-ins never start other birds.</p>
    </details>}
  </section>;
}
