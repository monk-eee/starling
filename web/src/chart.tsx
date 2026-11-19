import { useEffect, useRef } from "react";
import { init, use } from "echarts/core";
import type { EChartsType } from "echarts/core";
import { LineChart } from "echarts/charts";
import { GridComponent, TooltipComponent } from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";
import type { Point } from "./feed";

use([LineChart, GridComponent, TooltipComponent, CanvasRenderer]);

type Line = { name: string; color: string; value: (p: Point) => number | null };
export const throughput: Line[] = [
  { name: "Successful", color: "#68e5c4", value: p => (p.calls - p.failed) * 1000 / p.span_ms },
  { name: "Failed", color: "#fa9f88", value: p => p.failed * 1000 / p.span_ms },
];
export const percentiles: Line[] = [
  { name: "p50", color: "#8eb2ff", value: p => p.p50_ms },
  { name: "p95", color: "#b79bf8", value: p => p.p95_ms },
  { name: "p99", color: "#e5ba7b", value: p => p.p99_ms },
];
export const errors: Line[] = [
  { name: "Failed calls", color: "#fa9f88", value: p => p.calls ? 100 * p.failed / p.calls : null },
];

export function Chart({ title, unit, points, lines }: { title: string; unit: string; points: Point[]; lines: Line[] }) {
  const element = useRef<HTMLDivElement>(null);
  const chart = useRef<EChartsType | null>(null);
  useEffect(() => {
    const node = element.current!;
    const instance = init(node, undefined, { renderer: "canvas" });
    chart.current = instance;
    const observer = new ResizeObserver(() => instance.resize());
    observer.observe(node);
    return () => { observer.disconnect(); instance.dispose(); chart.current = null; };
  }, []);
  useEffect(() => {
    const axisLabel = { color: "#a6b3bc", fontSize: 10 };
    chart.current?.setOption({
      animation: false,
      grid: { top: 10, bottom: 24, left: 44, right: 12 },
      tooltip: { trigger: "axis", renderMode: "richText", confine: true },
      xAxis: { type: "category", data: points.map(p => `${p.second}s`), axisLabel, boundaryGap: false,
        axisLine: { lineStyle: { color: "#34434d" } } },
      yAxis: { type: "value", min: 0, axisLabel, splitLine: { lineStyle: { color: "#2b373f" } } },
      series: lines.map(l => ({ name: l.name, type: "line", showSymbol: points.length <= 1,
        symbolSize: 5, lineStyle: { width: 2 }, itemStyle: { color: l.color },
        data: points.map(p => { const v = l.value(p); return v === null ? null : Number(v.toFixed(3)); }) })),
    });
  }, [points, lines]);
  return <section className="panel graph">
    <header><h2>{title}</h2><small>{unit}</small></header>
    <div className="legend">{lines.map(l => <span key={l.name}><i style={{ background: l.color }} />{l.name}</span>)}</div>
    <div ref={element} className="canvas" role="img" aria-label={`${title} by elapsed second`} />
  </section>;
}
