"""Replay the frozen component with the native CPU sampler binding repaired."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import sys
import tarfile
import tempfile

import native
import replay

ID = "ordinary-app-plan-reuse-sampler-retry"
ORIGINAL = "045923bf9c938404d250b535e2833f49bc543e1edafb28fcc801c04a245f5315"
FAILED = "266c6ba40ac1892a1b34b3af6f23fda0c7888566b9e765d7f04489f1d0861f99"
PACKET = native.HERE / "request-sampler-retry.json"
MEASURE = native.HERE / "measure.py"
ARM_PATH = str(MEASURE.relative_to(native.ROOT))
ATTEMPT = "native-02-sampler-retry"
original_check = native.check


def corrected_measure():
    source = MEASURE.read_bytes()
    for old, new in (
        (b"for worker, offset in zip(workers, (0, 2), strict=True):",
         b"for worker_pid, offset in zip(workers, (0, 2), strict=True):"),
        (b"if b[-1] == worker):", b"if b[-1] == worker_pid):"),
    ):
        native.run.require(source.count(old) == 1, "sampler retry source differs from original")
        source = source.replace(old, new)
    return source


def check():
    native.run.require(native.run.digest(native.HERE / "request.json") == ORIGINAL,
                       "original request seal differs")
    previous = native.TARGET / "native-01/raw.tar.gz"
    native.run.require(native.run.digest(previous) == FAILED, "failed raw receipt differs")
    native.REQUEST = native.HERE / "request.json"
    try:
        original_check()
    finally:
        native.REQUEST = PACKET
    packet = native.run.read_json(PACKET)
    local = dict(native.pins())
    local[ARM_PATH] = hashlib.sha256(corrected_measure()).hexdigest()
    native.run.require(packet["id"] == ID and packet["local_sha256"] == local
                       and packet["retry_driver_sha256"] == native.run.digest(native.HERE / "native-sampler-retry.py"),
                       "sampler retry packet changed")
    assert packet["cpus"] == [0, 1]
    compile(corrected_measure(), str(MEASURE), "exec")
    return packet


def bundle(output):
    pins = native.run.read_json(PACKET)["local_sha256"]
    manifest = output / "INPUTS.sha256"
    manifest.write_text("".join(f"{sha}  {name}\n" for name, sha in pins.items()))
    destination = output / "input.tar"
    with tarfile.open(destination, "x") as archive:
        archive.add(manifest, arcname=manifest.name)
        for name in pins:
            data = corrected_measure() if name == ARM_PATH else (native.ROOT / name).read_bytes()
            native.run.require(hashlib.sha256(data).hexdigest() == pins[name], f"retry bundle changed: {name}")
            item = tarfile.TarInfo(name)
            item.size = len(data)
            archive.addfile(item, io.BytesIO(data))
    return destination


def summarize(raw):
    pins = native.run.read_json(PACKET)["local_sha256"]
    with tempfile.TemporaryDirectory(prefix="staged-replay-", dir=raw.parent) as scratch:
        staged = Path(scratch)
        with tarfile.open(raw.with_name("input.tar")) as archive:
            native.run.require(sorted(archive.getnames()) == sorted(["INPUTS.sha256", *pins]),
                               "sampler retry archive membership differs")
            archive.extractall(staged, filter="data")
        for name, sha in pins.items():
            native.run.require(native.run.digest(staged / name) == sha, f"retry replay input differs: {name}")
        original_root, replay.ROOT = replay.ROOT, staged
        try:
            return replay.summarize(raw)
        finally:
            replay.ROOT = original_root


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument("action", choices=("check", "dispatch", "replay"))
    cli.add_argument("raw", nargs="?", type=Path, default=native.TARGET / ATTEMPT / "raw.tar.gz")
    args = cli.parse_args()
    check()
    if args.action == "dispatch":
        native.ID = ID
        native.check = check
        native.bundle = bundle
        native.summarize = summarize
        native.dispatch(ATTEMPT)
    elif args.action == "replay":
        result = summarize(args.raw.resolve())
        print(json.dumps(result, indent=2, sort_keys=True, allow_nan=False))
        if result["verdict"] == "unqualified":
            sys.exit(1)


if __name__ == "__main__":
    main()
