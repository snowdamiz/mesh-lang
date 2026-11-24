"""Recover the exact staged inputs and retained runner pins from the failed attempt."""
import hashlib
from io import BytesIO
import json
from pathlib import Path
import sys
import tarfile

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "replay"))
import controller as original

FAILED = "exit-bulk-d2-c7c50e726ca0"
FAILED_DIR = HERE.parent / "evidence" / FAILED
CANDIDATE = f"{original.NATIVE}/builds/{FAILED}"
ELF = "972514efac688fd029821152c3bb70a7e62bf1c6227ab8b2d061dc69c47a2dc7"
MANIFEST = "ab09bc77affff4496350202150dc6eb0b90d5c252f0a7552767e0070d615fe31"
FAILED_RAW = "6456252306fa0531d1de8dd878a6d328750bd7e77e555e5f12ff150f2f55d741"
PROBE = "replay/staged-relay-check.py"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def failed(packet):
    path = FAILED_DIR / "raw.tar.gz"
    assert original.digest(path) == FAILED_RAW
    assert original.run.read_json(FAILED_DIR / "status.json")["exit_code"] == 1
    assert original.run.read_json(FAILED_DIR / "controller-release.json")["exit_status"] == 1
    with tarfile.open(path) as archive:
        read = lambda name: original.raw_bytes(archive, name)
        candidate, baseline = read("runner-build.json"), read("baseline-build.json")
        assert sha(candidate) == MANIFEST and sha(baseline) == original.RETAINED_MANIFEST
        build = json.loads(candidate)
        assert build["files"]["mesh"] == ELF
        assert (build["architecture"], build["system"]) == ("x86_64", "Linux")
        assert build["combined_source_sha256"] == packet["source_archive_sha256"]
        assert build["d2_patch_sha256"] == packet["d2_patch_sha256"]
        assert json.loads(read("host.json")) == original.earlier_proof()["host"]
        assert f"Compiled: {CANDIDATE}/mesh" in read("runner-link.log").decode()
        log = read("faults-d2.log").decode()
        assert 'faults.py", line 117, in exercise' in log and "AttributeError: 'Relay' object has no attribute 'requests'" in log
    return candidate, baseline


def shipped():
    entries = {f"replay/{name}": ((HERE.parent / "replay" / name).read_bytes(),
                                 0o755 if name in {"combined-run", "link-candidate.sh"} else 0o644)
               for name in original.SHIPPED}
    entries[PROBE] = ((HERE / "staged-relay-check.py").read_bytes(), 0o644)
    return entries


def shipped_pins():
    return {name: sha(data) for name, (data, _) in shipped().items()}


def package(packet):
    # The old-package control uses the exact bytes handed to original SETUP.
    old = original.package_stage()
    assert sha(old) == packet["stage_sha256"]
    with tarfile.open(fileobj=BytesIO(old)) as archive:
        outer = {item.name: (archive.extractfile(item).read(), item.mode)
                 for item in archive if not item.name.startswith("replay/")}
    outer.update(shipped())
    outer["stage.py"] = ((HERE / "stage.py").read_bytes(), 0o644)
    outer["shipped.json"] = ((json.dumps(shipped_pins(), sort_keys=True) + "\n").encode(), 0o644)
    built = BytesIO()
    with tarfile.open(fileobj=built, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, (data, mode) in sorted(outer.items()):
            member = tarfile.TarInfo(name)
            member.mode, member.size = mode, len(data)
            archive.addfile(member, BytesIO(data))
    return old, built.getvalue()
