"""Check a built Starling CLI against its checkout and a synthetic loopback fixture."""
import argparse
import json
import os
from pathlib import Path
import queue
import re
import subprocess
import tempfile
import threading
import tomllib
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[1]
BUNDLE = {"index.html", "run.json", "summary.json", "timeseries.json", "operations.csv",
          "errors.csv", "junit.xml"}


def check_version(cli, tag, env):
    def toml(name):
        with (ROOT / name).open("rb") as file:
            return tomllib.load(file)

    package = toml("Cargo.toml")["package"]
    locked, = [p for p in toml("Cargo.lock")["package"]
               if p["name"] == package["name"] and "source" not in p]
    web = json.loads((ROOT / "web/package.json").read_bytes())
    lock = json.loads((ROOT / "web/package-lock.json").read_bytes())
    version = package["version"]
    if not all(p["version"] == version for p in (locked, web, lock, lock["packages"][""])):
        raise ValueError("Starling's CLI and dashboard versions disagree across manifests/lockfiles")
    actual = subprocess.check_output([cli, "--version"], text=True, env=env, timeout=15).strip()
    if actual != f"starling {version}" or (tag and tag != f"v{version}"):
        raise ValueError(f"expected starling {version}; binary={actual!r}, tag={tag!r}")
    print(f"Verified {actual}" + (f" against {tag}" if tag else ""), flush=True)


def check_workload(cli, target, work, reports, name, minimum, should_pass, env):
    config = work / f"{name}.yaml"
    config.write_text(f"""run: {{ name: Release smoke, duration: 1s, seed: 1 }}
target: {{ url: "{target}" }}
load: {{ users: 1, threads: 1, wait: 100ms, timeout: 5s }}
tasks: [{{ tool: echo, weight: 1, arguments: {{ text: hello }}, assert_exists: /content/0/text }}]
assert: {{ error_rate: "<= 0%", p95: "< 10s", p99: "< 10s", min_requests: {minimum} }}
""", encoding="utf-8")
    if should_pass:
        out = reports / "conformance"
        subprocess.run([cli, "check", "--config", config, "--all-protocols", "--report", out],
                       cwd=work, env=env, check=True, timeout=60)
        evidence = json.loads((out / "conformance.json").read_bytes())
        if evidence["verdict"] != "pass" or not evidence["profiles"] or not all(
                p["verdict"] == "pass" and p["requested"] == p["negotiated"]
                for p in evidence["profiles"]):
            raise RuntimeError("conformance exit and per-revision evidence disagree")

    out = reports / name
    result = subprocess.run([cli, "run", "--config", config, "--headless", "--report", out],
                            cwd=work, env=env, timeout=40)
    if result.returncode != (0 if should_pass else 1):
        raise RuntimeError(f"{name}: unexpected workload exit {result.returncode}")
    missing = {f for f in BUNDLE if not (out / f).is_file() or not (out / f).stat().st_size}
    if missing:
        raise RuntimeError(f"{name}: missing or empty report artifacts: {sorted(missing)}")
    summary = json.loads((out / "summary.json").read_bytes())
    gate = summary["acceptance"]
    checks = {c["metric"]: c["passed"] for c in gate["checks"]}
    junit, = ET.parse(out / "junit.xml").findall(".//testcase[@name='performance.acceptance']")
    if (not summary["calls"]["count"] or summary["calls"]["failed"] != 0
            or gate["status"] != ("passed" if should_pass else "failed")
            or checks != dict(error_rate=True, p95=True, p99=True, min_requests=should_pass)
            or (junit.find("failure") is None) != should_pass):
        raise RuntimeError(f"{name}: CLI, sample counts, gate rules or JUnit disagree: {gate}")
    print(f"{name}: {summary['calls']['count']} calls, exit {result.returncode}, {gate['status']}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("reports", type=Path, help="new directory to keep the synthetic reports")
    parser.add_argument("--tag", help="release tag to match, e.g. v0.2.0")
    args = parser.parse_args()
    cli, reports = args.binary.resolve(strict=True), args.reports.resolve()
    env = {k: v for k, v in os.environ.items() if not k.startswith(("STARLING_", "OTEL_"))}
    check_version(cli, args.tag, env)
    reports.mkdir(parents=True, exist_ok=False)
    with tempfile.TemporaryDirectory(prefix="starling-gates-") as tmp:
        work = Path(tmp)
        with subprocess.Popen([cli, "fixture", "--listen", "127.0.0.1:0"], cwd=work, env=env,
                              text=True, stdout=subprocess.PIPE) as fixture:
            try:
                line = queue.Queue()
                threading.Thread(target=lambda: line.put(fixture.stdout.readline()), daemon=True).start()
                try:
                    announced = line.get(timeout=15).strip()
                except queue.Empty as error:
                    raise RuntimeError("fixture did not announce an endpoint within 15 seconds") from error
                match = re.fullmatch(r"Fixture: (http://127\.0\.0\.1:\d+/mcp)", announced)
                if not match:
                    raise RuntimeError(f"fixture did not announce a loopback endpoint: {announced!r}")
                for name, minimum, passed in (("accepted", 1, True), ("rejected", 1000, False)):
                    check_workload(cli, match[1], work, reports, name, minimum, passed, env)
            finally:
                if fixture.poll() is None:
                    fixture.terminate()
                    try:
                        fixture.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        fixture.kill()


if __name__ == "__main__":
    main()
