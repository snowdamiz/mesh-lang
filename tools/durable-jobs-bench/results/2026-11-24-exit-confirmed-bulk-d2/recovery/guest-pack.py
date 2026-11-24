import json
from pathlib import Path
import shutil
import stat
import subprocess
import sys

root = Path.cwd()
here = root / "tools/durable-jobs-bench/results/2026-11-24-exit-confirmed-bulk-d2/recovery"
work = root / "target/short-exit-bulk-d2/recovery"
sys.path.insert(0, str(root / "proof/jobs-parity/linux-regression"))
from pack import add
sys.path.insert(0, str(here))
from inputs import PROBE, original, package, sha, shipped_pins

packet = original.pins()
old, recovery = package(packet)
(work / "recovery.tar").write_bytes(recovery)
archive = bytearray()
def directory(name):
    add(archive, name, stat.S_IFDIR | 0o755)
def put(name, data, mode=0o644):
    add(archive, name, stat.S_IFREG | mode, data)

put("init", (work / "init").read_bytes(), 0o755)
directory("relay-regression")
directory("relay-regression/certs")
parts = shipped_pins()
pins = dict(old=sha(old), recovery=sha(recovery), source=packet["source_archive_sha256"],
            original_shipped={key: value for key, value in parts.items() if key != PROBE},
            repaired_shipped=parts)
put("relay-regression/pins.json", json.dumps(pins).encode())
put("relay-regression/old.tar", old)
put("relay-regression/recovery.tar", recovery)
for name in ("stage.py", "staged-relay-check.py", "guest-init.py"):
    put("relay-regression/" + name, (here / name).read_bytes())

certs = work / "temporary-certs"
certs.mkdir(mode=0o700)
try:
    subprocess.run(["openssl", "req", "-x509", "-nodes", "-newkey", "rsa:2048", "-days", "1",
                    "-subj", "/CN=staged-relay", "-out", str(certs / "server.crt"),
                    "-keyout", str(certs / "server.key")], check=True, capture_output=True)
    certificate = (certs / "server.crt").read_bytes()
    for name, data in (("cert.pem", certificate), ("server.crt", certificate),
                       ("server.key", (certs / "server.key").read_bytes())):
        put("relay-regression/certs/" + name, data)
finally:
    shutil.rmtree(certs)
add(archive, "TRAILER!!!", 0)
assets = root / "target/linux-queue-perf"
with (work / "guest.cpio").open("wb") as out, (assets / "slow-final-4k-01.cpio").open("rb") as base:
    shutil.copyfileobj(base, out)
    out.write(archive)
