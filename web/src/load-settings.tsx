import { useState } from "react";
import type { FormEvent } from "react";
import { reportFormat, type ReportFormat } from "./feed";

export type LoadSettings = {
  users: number; threads: number; duration_ms: number; wait_ms: number; timeout_ms: number;
};
export type StartOptions = Partial<LoadSettings> & { endpoint?: string; protocol?: string; report_formats?: ReportFormat[] };

const durationUnits = [
  { ms: 86400000, label: "days" }, { ms: 3600000, label: "hours" },
  { ms: 60000, label: "minutes" }, { ms: 1000, label: "seconds" },
];
const unitFor = (ms: number) => durationUnits.find(u => ms >= u.ms && ms % u.ms === 0)?.ms ?? 1000;
export function durationLabel(ms: number) {
  const unit = unitFor(ms);
  return `${ms / unit}${unit === 86400000 ? "d" : unit === 3600000 ? "h" : unit === 60000 ? "m" : "s"}`;
}

function Slider({ id, label, initial, hint }: { id: string; label: string; initial: number; hint: string }) {
  const [value, setValue] = useState(initial);
  return <div className="load-field">
    <div><label htmlFor={id}>{label}</label><output htmlFor={id}>{value}</output></div>
    <input id={id} name={id} type="range" min="1" max="8" value={value}
      aria-describedby={`${id}-hint`} onChange={e => setValue(e.currentTarget.valueAsNumber)} />
    <small id={`${id}-hint`}>{hint}</small>
  </div>;
}

export function LoadForm({ initial, maxDurationMs, disabled, endpoint, endpoints, protocol, revisions, initialFormats, onStart, onCheck }: {
  initial: LoadSettings; maxDurationMs: number; disabled: boolean; endpoint: string; endpoints: string[];
  protocol: string | undefined; revisions: string[] | undefined;
  initialFormats: ReportFormat[] | undefined;
  onStart: (options: StartOptions) => void;
  onCheck: (options: {endpoint?: string; protocol?: string; all_protocols: boolean}) => void;
}) {
  const [unit, setUnit] = useState(() => unitFor(initial.duration_ms));
  const [formats, setFormats] = useState(initialFormats ?? []);
  function submit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    const data = new FormData(e.currentTarget);
    const options: StartOptions = {};
    const choice = data.get("endpoint");
    if (typeof choice === "string" && choice !== endpoint) options.endpoint = choice;
    const revision = data.get("protocol");
    const button = e.nativeEvent instanceof SubmitEvent ? e.nativeEvent.submitter : null;
    if (button instanceof HTMLButtonElement && button.name === "check") {
      const all = button.value === "all";
      onCheck({ ...options, ...(!all && typeof revision === "string" && { protocol: revision }), all_protocols: all });
      return;
    }
    if (typeof revision === "string" && revision !== protocol) options.protocol = revision;
    if (initialFormats && (formats.length !== initialFormats.length || formats.some(f => !initialFormats.includes(f)))) {
      options.report_formats = formats;
    }
    for (const key of ["users", "threads", "duration_ms", "wait_ms", "timeout_ms"] as const) {
      const value = Number(data.get(key));
      const effective = key === "duration_ms" ? Math.round(value * unit) : value;
      if (effective !== initial[key]) options[key] = effective;
    }
    onStart(options);
  }
  return <form id="load-settings" onSubmit={submit}>
    <fieldset className="panel load-panel" disabled={disabled}>
      <legend>Load settings</legend>
      <div className="load-grid">
        {endpoints.length > 0 && <div className="load-field"><label htmlFor="endpoint">Endpoint</label>
          <select id="endpoint" name="endpoint" defaultValue={endpoint}>{["default", ...endpoints].map(e =>
            <option key={e}>{e}</option>)}</select><small>One YAML target per run</small></div>}
        {protocol && revisions && <div className="load-field"><label htmlFor="protocol">MCP protocol revision</label>
          <select id="protocol" name="protocol" defaultValue={protocol} aria-describedby="protocol-hint">
            <option value="auto">Auto (negotiate)</option>
            {revisions.map(v => <option key={v}>{v}</option>)}
          </select><small id="protocol-hint">Choose a date to require that exact revision.</small></div>}
        <Slider id="users" label="Concurrent users" initial={initial.users} hint="One MCP session per user." />
        <Slider id="threads" label="Worker threads" initial={initial.threads} hint="Starling threads, not server threads." />
        <div className="load-field"><label htmlFor="duration_ms">Duration</label>
          <div className="duration-input">
            <input id="duration_ms" name="duration_ms" type="number" min={1000 / unit} max={maxDurationMs / unit}
              step={unit === 1000 ? "0.001" : "any"} required defaultValue={initial.duration_ms / unitFor(initial.duration_ms)} />
            <select aria-label="Duration unit" value={unit} onChange={e => setUnit(Number(e.currentTarget.value))}>
              {durationUnits.map(u => <option key={u.ms} value={u.ms}>{u.label}</option>)}
            </select>
          </div><small>1s to {durationLabel(maxDurationMs)}</small></div>
        <div className="load-field"><label htmlFor="wait_ms">Wait between calls (ms)</label>
          <input id="wait_ms" name="wait_ms" type="number" min="0" max="60000" required aria-describedby="wait-hint"
            defaultValue={initial.wait_ms} /><small id="wait-hint">0 = no pause. Add a delay only to pace each user.</small></div>
        <div className="load-field"><label htmlFor="timeout_ms">Operation timeout (ms)</label>
          <input id="timeout_ms" name="timeout_ms" type="number" min="50" max="15000" required
            defaultValue={initial.timeout_ms} /><small>50 to 15,000 ms per operation</small></div>
      </div>
      {initialFormats && <fieldset className="report-formats">
        <legend>Save report as</legend>
        {reportFormat.options.map(f => <label key={f}><input type="checkbox" checked={formats.includes(f)}
          disabled={formats.length === 1 && formats.includes(f)}
          onChange={e => setFormats(e.currentTarget.checked ? [...formats, f] : formats.filter(other => other !== f))} />
          {f === "junit" ? "JUnit XML" : f.toUpperCase()}</label>)}
        <small>Keep at least one. HTML saves the standalone Starling dashboard.</small>
      </fieldset>}
      {protocol && revisions && <div className="check-actions">
        <button type="submit" name="check" value="one" formNoValidate>Check revision</button>
        <button type="submit" name="check" value="all" formNoValidate>Check all revisions</button>
        <small>One read-only conformance pass. No tools are invoked and no load run starts.</small>
      </div>}
    </fieldset>
  </form>;
}

export function SettingsSummary({ settings }: { settings: LoadSettings }) {
  return <p className="panel load-summary"><strong>Load settings</strong>
    <span>{settings.users} concurrent users</span><span>{settings.threads} worker threads</span>
    <span>{durationLabel(settings.duration_ms)} duration</span><span>{settings.wait_ms === 0 ? "No wait between calls" : `${settings.wait_ms} ms wait`}</span>
    <span>{settings.timeout_ms} ms timeout</span>
  </p>;
}
