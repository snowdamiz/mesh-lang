"""Run the pinned product correctness or compiler performance proof with all three native locks."""
from datetime import datetime, timezone
import fcntl
from hashlib import sha256
import json
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parents[1]))
import run
from freeze import PREVIOUS, REV

QUEUE = run.ROOT / "target/linode-jobs-stepchange/requests"
CASES = ("short-set-grant", "mixed-checked-cursor", "ambiguous-claim-commit", "ambiguous-application-write",
         "teardown-release", "teardown-kill", "short-hot-grant", "hot-ambiguous-application-write")


def check(packet):
    for name, digest in packet["pins"].items():
        run.require(run.digest(run.ROOT / name) == digest, f"changed input {name}")
    source = HERE / "source.tar.gz"
    run.require(run.digest(source) == packet["archive_sha256"], "changed package")
    with tarfile.open(source) as archive:
        for name in ("compiler/mesh-rt/src/db/pg.rs", "compiler/mesh-rt/src/db/pool.rs"):
            exact = subprocess.check_output(["git", "show", f"{REV}:{name}"], cwd=run.ROOT)
            measured = subprocess.check_output(["git", "show", f"b3557273:{name}"], cwd=run.ROOT)
            run.require(exact == measured == archive.extractfile("source/" + name).read(), f"runtime differs at {name}")
        for name in ("proof", "semantic"):
            member = archive.extractfile(f"overlay/replay/{name}.py")
            compile(member.read(), name, "exec")
    old = (PREVIOUS / "native.sh").read_text()
    script = old.replace("e4156740c7409156409cd2f792f4875153d71040", REV).replace("e4156740", REV[:8])
    run.require(sha256(script.encode()).hexdigest() == packet["native_script_sha256"], "native launcher changed")
    subprocess.run(["bash", "-n"], input=script.encode(), check=True)
    return script


def verify(packet, path):
    with tarfile.open(path) as archive:
        def content(name):
            return archive.extractfile("./" + name).read()
        run.require(content("native.exit").strip() == b"0", "native program failed")
        if packet["mode"] == "standalone":
            actual = content("standalone-test.log")
            run.require(b"test the_performance_gate_measures_and_reports ... ok" in actual and
                        b"test result: ok. 1 passed; 0 failed" in actual, "native performance gate failed")
            return dict(native_test="passed", compiler=content("compiler.sha256").decode().split()[0])
        proof = {case: json.loads(content(f"semantic-root/semantic/{case}/result.json")) for case in CASES}
        mesh, witness = [json.loads(content(f"{key}.json")) for key in ("jobs", "witness")]
        for item in (mesh, witness):
            run.require(item["version"] == REV and item["source_sha256"] == packet["archive_sha256"]
                        and item["architecture"] == "x86_64" and item["system"] == "Linux", "wrong artifact")
        run.require(mesh["runtime_sha256"] == witness["runtime_sha256"] and mesh["files"]["mesh"] != witness["files"]["mesh"],
                    "runtime or witness differs")
        for key in ("short-set-grant", "short-hot-grant"):
            obs = proof[key]
            run.require(len(obs["app_inserts"]) == 4 and (obs["checked_app_probes"] < 4) == (key == "short-hot-grant"),
                        "checkout probes or writes differ")
        for key in ("ambiguous-application-write", "hot-ambiguous-application-write"):
            obs = proof[key]
            run.require(obs["wire"]["idle"] and obs["wire"]["receipt_dropped"] and obs["app_insert_calls"] == 3,
                        "lost receipt or subsequent write was mishandled")
        return dict(runtime=mesh["runtime_sha256"], elf=mesh["files"]["mesh"], cases=list(proof),
                    checked_probes=proof["short-set-grant"]["checked_app_probes"],
                    hot_probes=proof["short-hot-grant"]["checked_app_probes"])


def main(mode, action):
    run.require(mode in {"jobs", "standalone"} and action in {"check", "setup", "replay"}, "unknown proof")
    packet_path = HERE / f"{mode}.json"
    packet = run.read_json(packet_path)
    script = check(packet)
    if action == "check":
        return
    output = HERE / "evidence" / mode
    if action == "setup":
        with (run.SUITE / ".timing.lock").open("rb") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            events = [x.split() for x in (QUEUE / "STATUS.md").read_text().splitlines()
                      if x.split()[:1] in (["SETUP"], ["TIMING"], ["RELEASED"], ["HOLD"])]
            event = events[-1]
            run.require(len(event) == 4 and event[:2] == ["SETUP", packet["id"]]
                        and packet_path.read_bytes() == (QUEUE / f"{packet['id']}.json").read_bytes(),
                        "requires this exact controller grant")
            now = datetime.now(timezone.utc)
            left = (datetime.fromisoformat(event[3])-now).total_seconds() - 120
            run.require(datetime.fromisoformat(event[2]) <= now and left >= 600, "inactive or expiring native grant")
            output.mkdir(parents=True, exist_ok=False)
            run.write_json(output / "grant.json", dict(event=event, seal=run.digest(packet_path)))
            with ((HERE / "source.tar.gz").open("rb") as source, (output / "native.log").open("xb") as log):
                try:
                    result = run.ssh(["bash", "-c", script, "--", mode, packet["attempt"], REV, packet["archive_sha256"]],
                                     stdin=source, stdout=log, stderr=log, timeout=left)
                finally:
                    with (output / "raw.tar.gz").open("xb") as raw, (output / "export.log").open("xb") as export:
                        run.ssh(["tar", "-czf", "-", "-C", "/var/lib/durable-jobs-bench/native/runs/" + packet["attempt"], "."],
                                stdout=raw, stderr=export, timeout=105).check_returncode()
            result.check_returncode()
    receipt = verify(packet, output / "raw.tar.gz")
    receipt.update(source=REV, raw_sha256=run.digest(output / "raw.tar.gz"))
    print(json.dumps(receipt, indent=2, sort_keys=True))
    if action == "setup":
        run.write_json(output / "verified.json", receipt)
    else:
        run.require(receipt == run.read_json(output / "verified.json"), "recorded receipt differs")


if __name__ == "__main__":
    main(*sys.argv[1:])
