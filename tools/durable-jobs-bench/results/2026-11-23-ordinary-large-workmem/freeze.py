#!/usr/bin/env python3
"""Seal the work_mem-only launcher overlay and unchanged scoring dependencies."""

from copy import deepcopy
from hashlib import file_digest, sha256
import io
import json
from pathlib import Path
import sys
import tarfile

LANE = Path(__file__).resolve().parent
ROOT = LANE.parents[1]
ASSEMBLY = LANE.with_name("ordinary-trusted-large-assembly")
SUITE = ROOT / "tools/durable-jobs-bench"
INPUTS = LANE / "inputs"
BASE = "/var/lib/durable-jobs-bench/native"
LABELS = {"ordinary": ("ordinary", 16), "ordinary-4mb-a": ("ordinary", 4),
          "river-external": ("river-external", 16), "ordinary-4mb-b": ("ordinary", 4),
          "apalis-external": ("apalis-external", 16), "oban-external": ("oban-external", 16)}
DRIVERS = ("freeze.py", "series.py", "setup.py", "proof.py", "install.remote.sh", "local.py")


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text())


def write(path, document):
    with path.open("x") as stream:
        json.dump(document, stream, indent=2)
        stream.write("\n")


def replace_once(source, old, new, count=1):
    if source.count(old) != count:
        raise ValueError(f"stock shell anchor changed: {old}")
    return source.replace(old, new)


def ordinary_launcher():
    source = (INPUTS / "ordinary-run").read_text()
    source = replace_once(source, 'base=/var/lib/durable-jobs-bench/native\n', '''base=/var/lib/durable-jobs-bench/native
launcher="${1:?pinned Mesh arm}"
mb="${2:?work_mem in MiB}"
shift 2
[[ "$launcher" == "$base"/source-ordinary-trusted-large-*/tools/durable-jobs-bench/native/arm.py ]]
[[ "$mb" == 4 || "$mb" == 16 ]]
''')
    source = replace_once(source, 'cd "$here"', 'cd "$(dirname "$launcher")"')
    return replace_once(source, 'python3 -B "$here/arm.py"',
                        'python3 -B "$here/entry.py" mesh "$mb" "$launcher"', 2)


def oban_launcher():
    source = (INPUTS / "oban-run").read_text()
    source = replace_once(source, 'base=/var/lib/durable-jobs-bench/native\n', '''base=/var/lib/durable-jobs-bench/native
peer="${1:?pinned EXTERNAL overlay}"
shift
[[ "$peer" == "$base"/payload-storage-peers-*/overlay.py ]]
''')
    source = replace_once(source, 'exec 9>"$base/native.lock"', 'exec 9<"$base/native.lock"')
    return replace_once(source, 'python3 -B "$overlay/overlay.py" oban "$arm"',
                        'python3 -B "$overlay/entry.py" oban 16 "$arm" "$peer"')


def sources():
    files = {path.name: path.read_bytes() for path in (LANE / "overlay").iterdir() if path.suffix == ".py"}
    files.update({"mesh-run": ordinary_launcher().encode(), "oban-run": oban_launcher().encode()})
    return files


def donor():
    config = read(INPUTS / "original-external.json")
    if config != read(ASSEMBLY / "freeze-retry/external.json"):
        raise ValueError("donor inventory changed")
    for label, name in (("ordinary", "ordinary-run"), ("oban-external", "oban-run"),
                        ("river-external", "overlay.py")):
        lane = config["lanes"][label]
        spec = lane["inventory"]["engines"][lane["engine"]]
        original = spec["argv"][-2] if label != "river-external" else spec["argv"][2]
        if digest(INPUTS / name) != spec["verified_files"][original]:
            raise ValueError(f"local source disagrees with pinned native wrapper: {original}")
    if config["cpus"] != [0, 1] or config["profiles"] != ["large"]:
        raise ValueError("invalid frozen workload or CPU mask")
    return config


def configuration(install, files):
    source = donor()
    peers = source["lanes"]["river-external"]["inventory"]["engines"]["go"]["argv"]
    peer = peers[2]
    config = dict(source, description="Large EXTERNAL. Shared-process ordinary Mesh, Oban, River and Apalis at 16 MiB. Two otherwise identical ordinary 4 MiB controls.", lanes={})
    for label, (original, mb) in LABELS.items():
        lane = deepcopy(source["lanes"][original])
        engine = lane["engine"]
        spec = lane["inventory"]["engines"][engine]
        argv = spec["argv"]
        if original == "ordinary":
            assert argv[-1] == "mesh" and argv[-2].endswith("/native/run")
            launcher = str(Path(argv[-2]).with_name("arm.py"))
            argv = [*argv[:-2], f"{install}/mesh-run", launcher, str(mb), "mesh"]
        elif engine == "oban":
            assert argv[-2].endswith("/oban-run")
            argv = [*argv[:-2], f"{install}/oban-run", peer, argv[-1]]
        else:
            assert argv[:4] == ["/usr/bin/python3", "-B", peer, engine]
            argv = [*argv[:2], f"{install}/entry.py", engine, str(mb), argv[4], peer, *argv[5:]]
        spec["argv"] = argv
        spec["verified_files"].update({f"{install}/{name}": sha for name, sha in files.items()})
        if "*.json" not in spec["evidence_globs"]:
            spec["evidence_globs"] += ["workmem*.json"]
        config["lanes"][label] = lane
    return config


def dependencies():
    return [
        *(LANE / name for name in DRIVERS),
        *(path for path in sorted(INPUTS.iterdir()) if path.is_file()),
        *(ASSEMBLY / "replay-retry" / name for name in
          ("interleave.py", "receipt.py", "assemble.py", "freeze.py")),
        ASSEMBLY / "source-retry/tools/durable-jobs-bench/native/assembly-pins.json",
        ASSEMBLY / "inputs/storage-upstream.py", ASSEMBLY / "inputs/peer-storage-replay.py",
        *(SUITE / name for name in ("run.py", "verify.py", "worker.py")),
    ]


def stock_pins(config):
    pins = {}
    for label, (original, _) in LABELS.items():
        lane = config["lanes"][original]
        spec = lane["inventory"]["engines"][lane["engine"]]
        for path, sha in spec["verified_files"].items():
            if pins.setdefault(path, sha) != sha:
                raise ValueError(f"incompatible donor checksums for {path}")
    return pins


def freeze(folder):
    folder.mkdir()
    files = sources()
    archive = folder / "overlay.tar"
    with tarfile.open(archive, "x") as bundle:
        for name, data in sorted(files.items()):
            member = tarfile.TarInfo(name)
            member.size = len(data)
            member.mode = 0o755 if name.endswith("-run") else 0o644
            bundle.addfile(member, io.BytesIO(data))
    sha = digest(archive)
    members = {name: sha256(data).hexdigest() for name, data in files.items()}
    install = f"{BASE}/ordinary-large-workmem-{sha[:16]}"
    write(folder / "large.json", configuration(install, members))
    write(folder / "stock-pins.json", stock_pins(donor()))
    write(folder / "freeze.json", {
        "archive_sha256": sha, "install": install, "members": members,
        "series_sha256": digest(folder / "large.json"),
        "stock_pins_sha256": digest(folder / "stock-pins.json"),
        "drivers": {str(path.relative_to(ROOT)): digest(path) for path in dependencies()},
    })


def verify(folder):
    f = read(folder / "freeze.json")
    archive = folder / "overlay.tar"
    assert digest(archive) == f["archive_sha256"]
    assert digest(folder / "large.json") == f["series_sha256"]
    assert digest(folder / "stock-pins.json") == f["stock_pins_sha256"]
    assert {str(path.relative_to(ROOT)): digest(path) for path in dependencies()} == f["drivers"]
    with tarfile.open(archive) as bundle:
        members = bundle.getmembers()
        assert all(member.isfile() and "/" not in member.name for member in members)
        assert len(members) == len(f["members"])
        assert {member.name: sha256(bundle.extractfile(member).read()).hexdigest()
                for member in members} == f["members"]
    assert configuration(f["install"], f["members"]) == read(folder / "large.json")
    assert stock_pins(donor()) == read(folder / "stock-pins.json")
    return f


if __name__ == "__main__":
    action, name = sys.argv[1:]
    path = Path(name).resolve()
    if action == "freeze":
        freeze(path)
    elif action == "verify":
        print(json.dumps(verify(path), indent=2))
    else:
        raise SystemExit("use freeze or verify")
