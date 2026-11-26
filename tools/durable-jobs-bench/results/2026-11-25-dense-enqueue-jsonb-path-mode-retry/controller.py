"""Recheck the released failure and bind one mode-only timing retry."""

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import shlex
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
ORIGINAL = HERE.with_name(HERE.name.removesuffix("-mode-retry"))
ID = HERE.name
OUT = HERE / "evidence/native"
PACKET = HERE / "controller-packet.json"
ORIGINAL_SEAL = "9db1ec163a8f5fbd1a5708984769f808284cced01ac35c536b810fcc61443b5c"
FAILED_RAW = "a37bc3f4188ac44aa7721a69b1a26eaab9f0ecef5a194e0e9c88597da612346e"
FAILED_RELEASE = "4a31cce67147ab8ede38827d897de863e77e67e5ee5cfaa00052f92a0dac188c"
FAILED_STDERR = ("python3: can't open file '/var/tmp/dense-enqueue-jsonb-path.ovXSGqPL/"
                 "factorial_arm.py': [Errno 13] Permission denied\n")

sys.path.insert(0, str(ORIGINAL))
spec = importlib.util.spec_from_file_location("sealed_dense_enqueue", ORIGINAL / "controller.py")
parent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(parent)
sys.path.remove(str(ORIGINAL))
read, sha = parent.replay.read, parent.replay.sha


def failed_inputs():
    failure = ORIGINAL / "evidence/native"
    raw, = (failure / "blocks").glob("01-noop.*/warmup-stock-a/warmup-mesh/raw.tar.gz")
    release = failure / "controller/controller-release.json"
    assert raw.read_bytes() == (HERE / "evidence/original-warmup.tar.gz").read_bytes()
    assert release.read_bytes() == (HERE / "evidence/original-release.json").read_bytes()
    assert sha(raw) == FAILED_RAW and sha(release) == FAILED_RELEASE
    receipt = read(release)
    assert receipt["id"] == ORIGINAL.name and receipt["exit_status"] == 1 and receipt["groups"] == [1, 2]
    with tarfile.open(raw) as archive:
        assert archive.extractfile("adapter.log").read().decode() == FAILED_STDERR
        assert json.load(archive.extractfile("status.json"))["exit_code"] == 2
        assert not any(name.startswith("adapter/") for name in archive.getnames())
    return raw, release


def local_proof():
    old = (ORIGINAL / "launch.sh").read_bytes()
    patched = (HERE / "launch.sh").read_bytes()
    cursor = b' > "$stage/factorial_arm.py"\n'
    assert old.count(cursor) == 1 and patched == old.replace(
        cursor, cursor + b'chmod 0644 "$stage/factorial_arm.py"\n')
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    evidence = read(HERE / "evidence/local-launch.json")
    pins = evidence["input_sha256"]
    assert pins["original.sh"] == sha(ORIGINAL / "launch.sh") and pins["amended.sh"] == sha(HERE / "launch.sh")
    for path in parent.FILES:
        if path.name != "launch.sh":
            assert pins[path.name] == sha(path)
    rows = evidence["cases"]
    assert [(r["launcher"], r["umask"], r["treatment"], r["exit_code"]) for r in rows] == [
        ("original.sh", "077", "stock", 2), ("original.sh", "022", "stock", 0),
        ("amended.sh", "077", "stock", 0), ("amended.sh", "077", "projected", 0),
    ]
    assert rows[0]["stderr"].endswith("/factorial_arm.py': [Errno 13] Permission denied")
    for row in rows[1:]:
        assert row["entry"] == dict(effective_uid=65534, python_mode="0o644",
                                    disposition="stopped before product code",
                                    projected=(row["treatment"] == "projected"))


def command(action):
    return shlex.join(("python3", "-B", str((HERE / "controller.py").relative_to(ROOT)), action))


def prepare():
    assert sha(parent.PACKET) == ORIGINAL_SEAL
    baseline, original_config = parent.config()
    history = parent.preflight(baseline, original_config)
    sealed = parent.packet(history)
    assert read(parent.PACKET) == sealed, "original sealed inputs drifted"
    local_proof()
    failure = failed_inputs()

    inputs = (HERE / "README.md", HERE / "controller.py", HERE / "launch.sh", HERE / "prove_launch.py",
              HERE / "evidence/local-launch.json", HERE / "evidence/original-warmup.tar.gz",
              HERE / "evidence/original-release.json", parent.PACKET, *failure)
    amended = dict(sealed, id=ID, supersedes_attempt=ORIGINAL.name, original_seal=ORIGINAL_SEAL,
                   correction="Set mode 0644 on the root-generated Python entry before runuser.",
                   offline_check=command("check"), timing_command=command("timing"),
                   replay_command=command("replay"), evidence_destination=str(OUT),
                   controller_receipt_dir=str(OUT / "controller"),
                   local_sha256=dict(sealed["local_sha256"]) |
                   {str(path.relative_to(ROOT)): sha(path) for path in inputs})
    parent.ID, parent.OUT, parent.PACKET = ID, OUT, PACKET
    parent.FILES = tuple(HERE / "launch.sh" if path.name == "launch.sh" else path for path in parent.FILES)
    _, config = parent.config()
    for lane in parent.LANES:
        stock = original_config["lanes"][lane]
        expected = deepcopy(stock)
        engine = expected["inventory"]["engines"]["mesh"]
        old_stage = str(parent.run.REMOTE / f"{ORIGINAL.name}-{lane}")
        new_stage = str(parent.run.REMOTE / f"{ID}-{lane}")
        engine["argv"] = [arg.replace(old_stage + "/", new_stage + "/") for arg in engine["argv"]]
        engine["verified_files"] = {path.replace(old_stage + "/", new_stage + "/"):
                                    (sha(HERE / "launch.sh") if path == old_stage + "/launch.sh" else value)
                                    for path, value in engine["verified_files"].items()}
        assert config["lanes"][lane] == expected, lane
    return amended, config


def main(args):
    expected, cfg = prepare()
    if args == ["seal"]:
        with PACKET.open("x") as stream:
            json.dump(expected, stream, indent=2, sort_keys=True)
            stream.write("\n")
    assert read(PACKET) == expected, "amendment differs from the exact seal"
    if args in (["seal"], ["check"]):
        print(sha(PACKET))
        return 0
    if args == ["replay"]:
        print(json.dumps(parent.replay_native(cfg), indent=2, sort_keys=True, allow_nan=False))
        return 0
    parent.bind_dispatch()
    if args == ["timing"]:
        return parent.factorial.dispatch(cfg, False)
    if len(args) == 2 and args[0] == "intake":
        return parent.factorial.intake(cfg, args[1], False)
    raise SystemExit("Use check, replay, or controller-only intake PACKET_SHA.")


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
