"""Collect and gate first-party coverage. Run from a checkout with the dashboard dependencies installed."""

import argparse
from pathlib import Path
import shutil
import subprocess
from tempfile import TemporaryDirectory

ROOT = Path(__file__).resolve().parents[1]
FLOORS = {"rust": {"lines": 90}, "dashboard": {"lines": 90, "branches": 90}}


def sources(suite):
    if suite == "rust":
        src = ROOT / "src"
        paths = (p for p in src.rglob("*.rs") if p.name != "tests.rs" and "tests" not in p.relative_to(src).parts)
    else:
        paths = (p for p in (ROOT / "web/src").rglob("*") if p.suffix in {".ts", ".tsx"} and not p.name.endswith(".d.ts"))
    return {p.relative_to(ROOT).as_posix() for p in paths}


def summarize(suite, lcov, expected):
    rows = {}
    normalized = []
    for record in lcov.split("end_of_record"):
        fields = record.strip().splitlines()
        if not fields:
            continue
        [source] = [v[3:].replace("\\", "/") for v in fields if v.startswith("SF:")]
        if source not in expected or source in rows:
            raise ValueError(f"Unexpected or duplicate coverage source: {source}")
        lines = [v[3:].split(",")[:2] for v in fields if v.startswith("DA:")]
        hits = {int(n): int(count) for n, count in lines}
        if len(hits) != len(lines) or any(n <= 0 or count < 0 for n, count in hits.items()):
            raise ValueError(f"Invalid line counters in {source}")
        covered = sum(count > 0 for count in hits.values())
        branches = [v.rsplit(",", 1)[1] for v in fields if v.startswith("BRDA:")]
        rows[source] = {"lines": (covered, len(hits)), "branches": (sum(v != "-" and int(v) > 0 for v in branches), len(branches))}
        # LLVM's LF/LH can count overlapping functions more than once. Count each physical DA line once.
        vals = [v for v in fields if not v.startswith(("LF:", "LH:"))]
        normalized += [f"SF:{source}" if v.startswith("SF:") else v for v in vals]
        normalized += [f"LF:{len(hits)}", f"LH:{covered}", "end_of_record"]
    if not expected or rows.keys() != expected:
        raise ValueError(f"Missing coverage sources: {sorted(expected - rows.keys())}")

    metrics = FLOORS[suite]
    header = ["Source", *metrics]
    table = [f"## {suite.capitalize()} coverage", "", " | ".join(header), " | ".join("---" for _ in header)]
    totals = {m: tuple(map(sum, zip(*(r[m] for r in rows.values())))) for m in metrics}
    for name, row in [*sorted(rows.items()), ("**Total**", totals)]:
        cells = [f"{hit}/{total} ({hit / total:.2%})" if total else "0/0 (n/a)" for hit, total in (row[m] for m in metrics)]
        table.append(" | ".join([name, *cells]))
    failures = []
    for metric, minimum in metrics.items():
        hit, total = totals[metric]
        if not total or 100 * hit < minimum * total:
            failures.append(f"{metric}: {hit}/{total} does not meet {minimum}%")
    table += ["", "Minimums: " + ", ".join(f"{n}% {m}" for m, n in metrics.items()) + "."]
    return "\n".join(normalized) + "\n", "\n".join(table) + "\n", failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=FLOORS)
    suite = parser.parse_args().suite
    out = ROOT / "target/coverage" / suite
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    with TemporaryDirectory(prefix="starling-coverage-") as work:
        raw = Path(work) / "lcov.info"
        if suite == "rust":
            subprocess.run([
                "cargo", "llvm-cov", "--locked", "--workspace", "--all-features", "--remap-path-prefix",
                "--ignore-filename-regex", r"(^|[/\\])(tests([/\\]|\.rs$)|target[/\\])",
                "--lcov", "--skip-functions", "--output-path", raw,
            ], cwd=ROOT, check=True)
        else:
            subprocess.run(["node", ROOT / "web/tests/coverage.mjs", work], cwd=ROOT, check=True)
        normalized, summary, failures = summarize(suite, raw.read_text(encoding="utf-8"), sources(suite))
        (out / "lcov.info").write_text(normalized, encoding="utf-8")
        (out / "summary.md").write_text(summary, encoding="utf-8")
        if suite == "dashboard":
            shutil.copytree(Path(work) / "html", out / "html")
    print(summary)
    if failures:
        raise SystemExit("Coverage gate failed: " + "; ".join(failures))


if __name__ == "__main__":
    main()
