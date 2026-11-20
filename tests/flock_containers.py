"""Verify the telemetry-only RPC on an isolated Docker network. Requires openssl and Docker."""
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
        finally:
            for cid in reversed(containers):
                command("docker", "rm", "-f", cid)
            command("docker", "network", "rm", network)


if __name__ == "__main__":
    check(sys.argv[1], sys.argv[2])
