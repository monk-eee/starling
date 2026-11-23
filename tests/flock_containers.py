"""Verify flock mTLS and HTTP/HTTPS MCP targets on an isolated Docker network."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid


def command(*args, timeout=30):
    p = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
    if p.returncode:
        raise RuntimeError(f"{args[0]} {args[1]} failed: {p.stderr.strip()} {p.stdout.strip()}")
    return p.stdout.strip()


def check(image, probe_image):
    network = "starling-check-" + uuid.uuid4().hex[:12]
    containers = []
    with tempfile.TemporaryDirectory(prefix="starling-test-pki-") as tmp:
        root = Path(tmp)
        ca, cakey = root / "ca.pem", root / "ca.key"
        command("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                "-subj", "/CN=Starling test CA", "-keyout", cakey, "-out", ca)

        def identity(name, role, serial):
            folder = root / name
            folder.mkdir()
            key, csr, cert, ext = (folder / f for f in ("key.pem", "request", "cert.pem", "ext"))
            command("openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
                    "-subj", f"/CN={name}", "-keyout", key, "-out", csr)
            ext.write_text(f"basicConstraints=CA:FALSE\nextendedKeyUsage={role}\nsubjectAltName=DNS:{name}\n")
            sign = ["-signkey", key] if name == "untrusted" else ["-CA", ca, "-CAkey", cakey]
            command("openssl", "x509", "-req", "-in", csr, *sign, "-set_serial", str(serial),
                    "-days", "1", "-extfile", ext, "-out", cert)
            args = []
            for var, filename, path in (("CA", "ca", ca), ("CERT", "cert", cert), ("KEY", "key", key)):
                args += ["--mount", f"type=bind,source={path},target=/pki/{filename},readonly",
                         "-e", f"STARLING_FLOCK_{var}=/pki/{filename}"]
            return args

        def create(net, extra, binary, args):
            uid = ["--user", f"{os.getuid()}:{os.getgid()}"] if hasattr(os, "getuid") else []
            cid = command("docker", "create", "--pull=never", "--network", net, "--read-only",
                          "--tmpfs", "/work:rw,mode=1777", "--cap-drop=ALL",
                          "--security-opt=no-new-privileges", *uid, *extra, binary, *args)
            containers.append(cid)
            command("docker", "start", cid)
            return cid

        def worker(name, tls):
            return create(network, tls, image, ["bird", "--id", name, "--home", "https://hub:8788", "--demo"])

        command("docker", "network", "create", "--internal", network)
        try:
            hub = create(network, ["--network-alias", "hub", *identity("hub", "serverAuth", 1)], image,
                         ["demo", "--web", "--listen", "127.0.0.1:8787", "--bird-listen", "0.0.0.0:8788"])
            probe = create("container:" + hub, ["--entrypoint", "node"], probe_image,
                           ["-e", "setInterval(()=>{},60000)"])
            def roster():
                js = "fetch('http://127.0.0.1:8787/api/v1/flock').then(r=>r.json()).then(v=>console.log(JSON.stringify(v.birds)))"
                return {b["id"]: b for b in json.loads(command("docker", "exec", probe, "node", "-e", js))}

            def until(predicate):
                end = time.monotonic() + 15
                while True:
                    birds = roster()
                    if predicate(birds):
                        return birds
                    if time.monotonic() >= end:
                        raise RuntimeError(f"flock condition not reached: {birds}")
                    time.sleep(.25)

            a = worker("wing-1", identity("one", "clientAuth", 2))
            worker("wing-2", identity("two", "clientAuth", 3))
            def live_pair(birds):
                return set(birds) == {"wing-1", "wing-2"} and all(
                    not b["stale"] and b["requests"] > 0 and b["failed"] == 0 and b["p95_ms"] is not None
                    for b in birds.values())

            until(live_pair)
            rejected = worker("unexpected", identity("untrusted", "clientAuth", 4))
            assert command("docker", "wait", rejected) != "0", "untrusted certificate was accepted"
            command("docker", "stop", "--timeout", "8", a)
            stale = until(lambda b: len(b) == 2 and b["wing-1"]["stale"] and not b["wing-2"]["stale"])
            assert stale["wing-1"]["requests"] > 0
            command("docker", "start", a)
            until(live_pair)
            print("Two independent Linux birds authenticated with mTLS: two live, one stale, two live again; no duplicate or untrusted bird.")

            results = root / "results"
            results.mkdir()
            results.chmod(0o777)
            collector = create(network, ["--network-alias", "check-hub", *identity("check-hub", "serverAuth", 5),
                               "--mount", f"type=bind,source={results},target=/evidence"], image,
                               ["collect", "--expect", "west,east", "--listen", "0.0.0.0:8790",
                                "--deadline-secs", "90", "--report", "/evidence/fleet"])
            deadline = time.monotonic() + 15
            while True:
                campaign = next((line.removeprefix("Check campaign: ") for line in command("docker", "logs", collector).splitlines()
                                 if line.startswith("Check campaign: ")), None)
                if campaign:
                    break
                if time.monotonic() > deadline:
                    raise RuntimeError("conformance collector did not announce its campaign")
                time.sleep(.2)
            workers = [create(network, identity(name, "clientAuth", serial), image,
                              ["check", "--demo", "--all-protocols", "--id", name, "--home", "https://check-hub:8790",
                               "--campaign", campaign, "--report", "/work/evidence"])
                       for name, serial in [("west", 6), ("east", 7)]]
            for cid in [*workers, collector]:
                assert command("docker", "wait", cid, timeout=100) == "0", command("docker", "logs", cid)
            combined = json.loads((results / "fleet/fleet-conformance.json").read_text())
            assert combined["campaign"] == campaign and combined["verdict"] == "pass" and not combined["missing_birds"]
            assert set(combined["workers"]) == {"west", "east"}
            for report in combined["workers"].values():
                assert {p["requested"] for p in report["profiles"]} == set(combined["expected_revisions"])
                assert all(p["verdict"] == "pass" for p in report["profiles"])
            print("Two separate containers submitted every selected conformance profile over mutual TLS.")

            target = create(network, ["--network-alias", "target", "--network-alias", "wrong-name"],
                            image, ["fixture", "--listen", "127.0.0.1:8081"])
            # Terminate TLS in front of the real MCP fixture; neither listener is published on the host.
            edge = create("container:" + target, [*identity("target", "serverAuth", 7), "--entrypoint", "node"],
                          probe_image, ["-e", """
const http = require('node:http'), https = require('node:https'), fs = require('node:fs');
function forward(req, res) {
  const upstream = http.request({host:'127.0.0.1', port:8081, path:req.url, method:req.method,
    headers:{...req.headers, host:'127.0.0.1:8081'}}, reply => {
      res.writeHead(reply.statusCode, reply.headers); reply.pipe(res);
    });
  upstream.on('error', error => res.destroy(error));
  res.on('close', () => upstream.destroy());
  req.pipe(upstream);
}
http.createServer(forward).listen(8080, () =>
  https.createServer({key:fs.readFileSync('/pki/key'), cert:fs.readFileSync('/pki/cert')}, forward)
    .listen(8443, () => console.log('ready')));
"""])
            deadline = time.monotonic() + 15
            while "ready" not in command("docker", "logs", edge).splitlines():
                if time.monotonic() > deadline:
                    raise RuntimeError("MCP test listeners did not start")
                time.sleep(.2)

            def target_cli(args, *, trust, expected, no_roots=False):
                mounts = ["--mount", f"type=bind,source={results},target=/evidence"]
                if trust:
                    mounts += ["--mount", f"type=bind,source={ca},target=/etc/ssl/certs/ca-certificates.crt,readonly"]
                elif no_roots:
                    mounts += ["-e", "SSL_CERT_FILE=/dev/null", "-e", "SSL_CERT_DIR=/no-roots"]
                cid = create(network, mounts, image, args)
                assert command("docker", "wait", cid, timeout=65) == str(expected), command("docker", "logs", cid)

            https_url = "https://target:8443/mcp"
            for name, url, trust, ok in [
                ("http", "http://target:8080/mcp", False, True),
                ("https", https_url, True, True),
                ("untrusted-ca", https_url, False, False),
                ("wrong-name", "https://wrong-name:8443/mcp", True, False),
            ]:
                revisions = ["--all-protocols"] if ok else ["--protocol", "2025-06-18"]
                target_cli(["check", "--url", url, *revisions, "--report", f"/evidence/{name}"],
                           trust=trust, expected=0 if ok else 1, no_roots=name == "http")
                report = json.loads((results / name / "conformance.json").read_bytes())
                assert report["verdict"] == ("pass" if ok else "inconclusive"), report
                if not ok:
                    assert report["profiles"][0]["checks"][0]["evidence"] == "transport_error", report

            (results / "case.yaml").write_text(f"""run: {{ name: TLS fixture, duration: 1s, seed: 1 }}
target: {{ url: "{https_url}" }}
load: {{ users: 1, threads: 1, wait: 100ms, timeout: 5s }}
tasks: [{{ tool: status, weight: 1, assert_exists: /content/0/text }}]
assert: {{ min_requests: 1 }}
""", encoding="utf-8")
            target_cli(["run", "--headless", "--config", "/evidence/case.yaml", "--report", "/evidence/tls-run"],
                       trust=True, expected=0)
            summary = json.loads((results / "tls-run/summary.json").read_bytes())
            assert summary["calls"]["failed"] == 0 and summary["acceptance"]["status"] == "passed"
            assert summary["run"]["target_origin"] == "https://target:8443"
            print("HTTP and HTTPS passed across containers. HTTPS workload passed; untrusted CA and wrong DNS name failed.")
        finally:
            for cid in reversed(containers):
                command("docker", "rm", "-f", cid)
            command("docker", "network", "rm", network)


if __name__ == "__main__":
    check(sys.argv[1], sys.argv[2])
