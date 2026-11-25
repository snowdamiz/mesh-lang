"""Reproduce both entries offline with the staged absolute paths as Linux nobody."""
import json
from pathlib import Path
import subprocess
import tarfile
from tempfile import TemporaryDirectory

from retry import FROZEN, FROZEN_ARCHIVE, HERE, HOST_PAYLOAD, ROOT, bind, original, require, run

IMAGE = "3dc01f342e31ad8de39338d14ab12de0090c9102b34f75ca9774ff1f4f5e904e"


def main():
    _, packet, broker = bind()
    config = original.make_config(packet, broker, "pilot")
    stage = Path(config["private_stage"])
    runner = "/var/lib/durable-jobs-bench/native/builds/trusted-jobs-product-623c46b92cc5"
    manifest = original.stock.MANIFEST
    broker_pin = broker["files"]["mesh"]
    with TemporaryDirectory(prefix="dense-entry-", dir=ROOT / "target") as temp:
        root = Path(temp)
        root.chmod(0o755)
        old, new = root / "packet", root / "packet-timing-splice"
        old.mkdir()
        new.mkdir()
        with tarfile.open(original.STAGE) as archive:
            for name in original.HOST_FILES:
                member = archive.extractfile(f"packet/{name}").read()
                require(original.dense.digest(member) == config["private_files"][str(stage / "packet" / name)],
                        f"wrong source for {name}")
                (old / name).write_bytes(member)
                if name not in HOST_PAYLOAD:
                    (new / name).write_bytes(member)
        for name in HOST_PAYLOAD:
            (new / name).write_bytes((HERE / name).read_bytes())
        harness = root / "frozen"
        with tarfile.open(FROZEN_ARCHIVE) as archive:
            archive.extractall(harness, members=[member for member in archive if member.name.startswith("ordinary-v2/")],
                               filter="data")
        (root / "archived-asset-digests.json").write_text(json.dumps({
            runner + "/mesh.json": manifest, str(stage / "broker/mesh"): broker_pin,
        }))
        args = ["docker", "run", "--rm", "--pull", "never", "--network", "none", "--read-only",
                "--user", "65534:65534", "--cpuset-cpus", "0,1",
                "--mount", f"type=bind,source={root},destination={stage},readonly",
                "--mount", f"type=bind,source={harness / 'ordinary-v2'},destination={FROZEN},readonly"]
        env = dict(PRODUCT_FROZEN_HARNESS=FROZEN, NATIVE_JOBS_ARTIFACTS=runner,
                   PRODUCT_MANIFEST_SHA256=manifest, DENSE_BROKER=stage / "broker/mesh",
                   DENSE_BROKER_SHA256=broker_pin)
        for key, value in env.items():
            args.extend(("-e", f"{key}={value}"))
        for width, directory in (("128", "packet"), ("128", "packet-timing-splice"),
                                 ("1024", "packet-timing-splice")):
            call = subprocess.run([*args, "-e", f"DENSE_WIDTH={width}", IMAGE, "python3", "-B",
                                   str(stage / "packet-timing-splice/entry_probe.py"),
                                   str(stage / directory / "adapter.py"), "--archived-asset-digests",
                                   str(stage / "archived-asset-digests.json")], capture_output=True, text=True)
            print(call.stdout + call.stderr, end="")
            if directory == "packet":
                require(call.returncode == 1 and call.stderr.endswith("ValueError: substring not found\n"),
                        "original entry failed differently")
            else:
                call.check_returncode()


if __name__ == "__main__":
    main()
