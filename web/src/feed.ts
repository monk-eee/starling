import { useEffect, useState } from "react";
import { z } from "zod";

export const api = "/api/v1/runs/local";
const latency = z.number().nonnegative().nullable();
const summary = z.object({
  count: z.number(), failed: z.number(), outcomes: z.record(z.string(), z.number()),
  p50_ms: latency, p95_ms: latency, p99_ms: latency,
});
const acceptance = z.object({
  status: z.enum(["passed", "failed", "incomplete"]),
  checks: z.array(z.object({
    metric: z.string(), expected: z.string(), unit: z.string(),
    observed: z.number().nullable(), passed: z.boolean().nullable(),
  })),
});
const point = z.object({
  second: z.number(), span_ms: z.number().positive(), calls: z.number(), failed: z.number(),
  p50_ms: latency, p95_ms: latency, p99_ms: latency,
});
const frame = z.object({
  schema_version: z.literal(1), sequence: z.number().int(),
  generation: z.number().int().optional(),
  run: z.object({
    name: z.string(), target_origin: z.string(), duration_ms: z.number(), users: z.number(),
    threads: z.number().int().optional(), wait_ms: z.number().optional(), timeout_ms: z.number().optional(),
    max_duration_ms: z.number().positive().optional(), bucket_secs: z.number().int().positive().optional(),
    requests: z.array(z.string()).optional(),
    endpoint: z.string().optional(), endpoint_names: z.array(z.string()).optional(),
    protocol: z.string().optional(), protocol_revisions: z.array(z.string()).optional(),
    assert: z.object({ error_rate: z.string().optional(), p95: z.string().optional(),
      p99: z.string().optional(), min_requests: z.number().optional() }).optional(),
  }),
  status: z.enum(["ready", "running", "stopping", "completed", "stopped", "failed"]),
  reason: z.string().nullable(), elapsed_ms: z.number(), sessions: z.number(), in_flight: z.number(),
  protocols: z.array(z.string()), calls: summary, operations: z.record(z.string(), summary),
  acceptance: acceptance.optional(),
  series: z.array(point), open_interval: point.optional(),
});
const message = frame.extend({ type: z.enum(["snapshot", "tick"]) });
export type Frame = z.infer<typeof frame>;
export type Point = z.infer<typeof point>;
export const terminal = (status: Frame["status"]) => ["completed", "stopped", "failed"].includes(status);

function savedReport() {
  const data = document.querySelector("#starling-report")?.textContent;
  if (!data) throw new Error("Missing Starling report data slot.");
  const value: unknown = JSON.parse(data);
  return value === null ? null : frame.parse(value);
}

export function subscribeLive(path: string, receive: (value: unknown, socket: WebSocket) => void,
  disconnected: () => void, invalid: () => void) {
  let disposed = false;
  let socket: WebSocket;
  let timer: ReturnType<typeof setTimeout>;
  let backoff = 500;
  function connect() {
    socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}${path}`);
    socket.onmessage = ({ data }: MessageEvent<string>) => {
      if (disposed) return;
      try { receive(JSON.parse(data), socket); backoff = 500; }
      catch { invalid(); socket.close(); }
    };
    socket.onclose = () => {
      if (disposed) return;
      disconnected();
      timer = setTimeout(connect, backoff);
      backoff = Math.min(5000, backoff * 2);
    };
  }
  connect();
  return () => { disposed = true; clearTimeout(timer); socket.close(); };
}

export function useFeed() {
  const [saved] = useState(savedReport);
  const [current, setCurrent] = useState<Frame | null>(saved);
  const [connection, setConnection] = useState(saved ? "Offline report" : "Connecting");
  const [error, setError] = useState("");

  useEffect(() => {
    if (saved) return;
    let last: Frame | null = null;
    return subscribeLive(`${api}/live`, (value, socket) => {
      const next = message.parse(value);
      if (next.type === "tick" && (!last || next.sequence !== last.sequence + 1)) {
        socket.send("snapshot");
        return;
      }
      const history = last?.series ?? [];
      last = { ...next, series: next.type === "snapshot" ? next.series : next.series.length ? [...history, ...next.series] : history };
      setCurrent(last);
      setConnection("Connected");
      setError("");
    }, () => setConnection("Reconnecting"), () => setError("The controller sent an unreadable or incompatible metrics frame."));
  }, [saved]);
  return { current, connection, offline: saved !== null, error };
}
