"""Enter a staged adapter as nobody, then stop before it can run an arm."""
import argparse
import hashlib
import inspect
import json
import os
from pathlib import Path
import runpy
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("adapter", type=Path)
    parser.add_argument("--archived-asset-digests", type=Path)
    args = parser.parse_args()
    assert os.getuid() == os.geteuid() == 65534 and os.getgid() == 65534
    adapter = args.adapter.resolve(strict=True)
    assert str(adapter).startswith("/var/lib/durable-jobs-bench/native/source-dense-producer-1024-")
    assert sorted(os.sched_getaffinity(0)) == [0, 1]
    sys.path.insert(0, str(adapter.parent))
    scope = runpy.run_path(str(adapter))
    arm = scope["arm"]
    original = Path(inspect.getsourcefile(arm.run_arm))
    if args.archived_asset_digests:
        # The local container has the exact staged source. ELF checks run on
        # the retained binaries during the controller's native readback.
        pins = json.loads(args.archived_asset_digests.read_text())
        digest = arm.h.digest

        def archived_digest(path):
            key = str(path)
            if key in pins:
                assert not Path(path).exists()
                return pins[key]
            return digest(path)

        arm.h.digest = archived_digest

    def entered():
        active = arm.run_arm
        assert active.__code__.co_filename == str(original)
        assert "capture_writer" in active.__code__.co_names
        assert "database_settings" in active.__code__.co_names
        assert active.__globals__["capture_writer"] is scope["capture_writer"]
        assert active.__globals__["Subject"] is scope["DenseSubject"]
        assert active.__globals__["Cluster"] is scope["Cluster"]
        assert arm.PROFILES["noop"][:4] == (2, 64, 16, 96)
        print(json.dumps(dict(
            adapter=str(adapter), adapter_sha256=sha(adapter), harness=str(original),
            harness_sha256=sha(original), uid=os.geteuid(), gid=os.getegid(),
            cpus=sorted(os.sched_getaffinity(0)), width=os.environ["DENSE_WIDTH"],
            asset_checks="archived-digests" if args.archived_asset_digests else "retained-files",
            entered_without_workload=True,
        ), sort_keys=True))

    arm.main = entered
    sys.argv = [str(adapter), "mesh", "noop", "/unused"]
    scope["main"]()


def sha(path):
    with path.open("rb") as f:
        return hashlib.file_digest(f, "sha256").hexdigest()


if __name__ == "__main__":
    main()
