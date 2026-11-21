import { z } from "zod";

const verdict = z.enum(["pass", "fail", "skip", "not_run", "unsupported", "inconclusive"]);
export const checkReport = z.object({
  schema_version: z.literal(1), suite: z.literal("mcp.streamable-http.core.v1"), bird_id: z.string(),
  pid: z.number(), verdict,
  profiles: z.array(z.object({
    requested: z.string(), negotiated: z.string().nullable(), specification: z.string().nullable(), verdict,
    checks: z.array(z.object({ rule: z.string(), status: verdict, evidence: z.string() })),
  })),
});
export type CheckReport = z.infer<typeof checkReport>;

export function Conformance({ report }: { report: CheckReport }) {
  return <section className="panel check-result" aria-label="MCP conformance report">
    <h2>MCP conformance: {report.verdict}</h2>
    <p>Streamable HTTP core profile · {report.bird_id}. Only the listed checks were assessed.</p>
    <a download="conformance.json" href={`data:application/json;charset=utf-8,${encodeURIComponent(JSON.stringify(report, null, 2))}`}>
      Download conformance JSON</a>
    {report.profiles.map(p => <div key={p.requested} className="table-wrap">
      <h3>{p.requested}: {p.verdict}</h3>
      <small>Negotiated: {p.negotiated ?? "none"}</small>
      <table><thead><tr><th>Rule</th><th>Result</th><th>Evidence</th></tr></thead>
        <tbody>{p.checks.map(c => <tr key={c.rule}><th scope="row">{c.rule}</th><td>{c.status.replaceAll("_", " ")}</td>
          <td>{c.evidence.replaceAll("_", " ")}</td></tr>)}</tbody></table>
    </div>)}
  </section>;
}
