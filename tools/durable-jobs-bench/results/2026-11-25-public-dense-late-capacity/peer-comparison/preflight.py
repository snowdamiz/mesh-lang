"""Verify stock inventory files and wrapper access under the native fence."""
from pathlib import Path
import json

PROGRAM = r"""
import hashlib, json, os, pathlib, subprocess, sys
inputs = json.loads(sys.argv[1])
assert pathlib.Path("/proc/sys/kernel/random/boot_id").read_text().strip() == inputs["host"][0]
assert os.uname().release == inputs["host"][1]
assert os.sched_getaffinity(0) == {0, 1, 2, 3}
result = {}
for label, spec in inputs["peers"].items():
    assert subprocess.check_output(["findmnt", "-nro", "MAJ:MIN", "--target", spec["storage"]], text=True).strip() == inputs["host"][2]
    files = {}
    for path, expected in spec["files"].items():
        with pathlib.Path(path).open("rb") as stream:
            actual = hashlib.file_digest(stream, "sha256").hexdigest()
        assert actual == expected, (label, path)
        files[path] = actual
    for flag in ("-f", "-r", "-x"):
        subprocess.run(["runuser", "-u", spec["user"], "--", "/usr/bin/test", flag, spec["wrapper"]], check=True)
    result[label] = dict(files=files, run_as=spec["user"], wrapper=spec["wrapper"],
                         inventory_sha256=spec["inventory_sha256"])
print(json.dumps(dict(passed=True, request_sha256=inputs["request_sha256"], host=inputs["host"], result=result),
                 indent=2, sort_keys=True))
"""


def attest(run, inventory_dir, engines, host, seal, log):
    specs = {}
    for label, engine in engines.items():
        path = inventory_dir / f"{label}.json"
        config = run.read_json(path)["engines"][engine]
        wrappers = [arg for arg in config["argv"] if arg in config["verified_files"] and Path(arg).name == "run"]
        wrapper, = wrappers
        specs[label] = dict(files=config["verified_files"], user="root" if label == "oban" else "nobody",
                            wrapper=wrapper, storage=config["postgres_storage"], inventory_sha256=run.digest(path))
    assert sum(len(spec["files"]) for spec in specs.values()) == 40
    document = dict(host=host, request_sha256=seal, peers=specs)
    run.ssh(["python3", "-B", "-c", PROGRAM, json.dumps(document)], stdout=log, timeout=75).check_returncode()


def verify_receipt(run, inventory_dir, engines, host, seal, path):
    receipt = run.read_json(path)
    assert receipt["passed"] and receipt["host"] == host and receipt["request_sha256"] == seal
    assert set(receipt["result"]) == set(engines)
    for label, engine in engines.items():
        source = inventory_dir / f"{label}.json"
        config = run.read_json(source)["engines"][engine]
        row = receipt["result"][label]
        assert row["inventory_sha256"] == run.digest(source)
        assert row["files"] == config["verified_files"]
        assert row["wrapper"] in config["argv"] and row["wrapper"] in row["files"]
        assert row["run_as"] == ("root" if label == "oban" else "nobody")
