#!/usr/bin/env python3
"""Build a Mesh executable with an optional observed GNU AArch64 section order."""
import argparse
from collections import Counter
from hashlib import sha256
import os
from pathlib import Path
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
ENTRY = Path(__file__).resolve()
sys.path.insert(0, str(ENTRY.parent))
from lib.mesh_section_layout import (
    Elf, Unsupported, match_map, observed_profile, read_bounded_json,
    require, run_link, write_json,
)


DRIVER_CONFIG = "_MESH_SECTION_LAYOUT_DRIVER"


def after_separator(args):
    return args[1:] if args and args[0] == "--" else args


def executable(name):
    path = shutil.which(name)
    require(path, f"executable not found: {name}")
    return os.path.abspath(path)


def build(args):
    driver = executable(args.cc_driver)
    compiler = executable(args.meshc)
    runtime = Path(os.path.abspath(args.runtime))
    require(runtime.is_file(), f"runtime archive not found: {runtime}")
    config = {"runtime": str(runtime), "driver": [driver, *args.cc_arg],
              "tail": args.cc_after, "zig": args.zig,
              "profile": str(args.profile.resolve()) if args.profile else None,
              "receipt": str(args.receipt.resolve()) if args.receipt else None}
    with tempfile.TemporaryDirectory(prefix="mesh-section-cc-") as directory:
        helper = Path(directory)
        (helper / "cc").symlink_to(ENTRY)
        config_path = helper / "config.json"
        write_json(config_path, config)
        env = dict(os.environ, PATH=os.pathsep.join((directory, os.environ.get("PATH", ""))),
                   MESH_RT_LIB_PATH=str(runtime), **{DRIVER_CONFIG: str(config_path)})
        return subprocess.call([compiler, "build", *after_separator(args.args)], env=env)


def driver(args):
    config = read_bounded_json(os.environ[DRIVER_CONFIG])
    command = [*config["driver"], *args, *config["tail"]]
    # The selected cc and its children see the original PATH.
    os.environ["PATH"] = os.environ["PATH"].split(os.pathsep, 1)[1]
    del os.environ[DRIVER_CONFIG]
    profile = Path(config["profile"]) if config["profile"] else None
    receipt = Path(config["receipt"]) if config["receipt"] else None
    options = argparse.Namespace(command=command, runtime=Path(config["runtime"]),
                                 profile=profile, receipt=receipt, cc=True)
    if not config["zig"] or not (profile or receipt) or args.count("-o") != 1:
        return run_link(options)

    # Zig cc cannot forward LLD's map flag. Capture the final LLD command
    # from its successful stock link, then require a byte-identical baseline.
    result = subprocess.run([*command, "-v"], capture_output=True)
    if result.returncode:
        sys.stderr.buffer.write(result.stderr)
        return result.returncode
    destination = args[args.index("-o") + 1]
    try:
        lines = result.stderr.decode().splitlines()
        recipes = [shlex.split(line) for line in lines if line.startswith("ld.lld ")]
        link, = [argv for argv in recipes if "-o" in argv and argv[argv.index("-o") + 1] == destination]
        stock = Path(destination).read_bytes()
    except (ValueError, OSError, IndexError) as error:
        reason = f"could not capture Zig's LLD link: {error}"
        if receipt:
            write_json(receipt, {"state": "stock", "reason": reason})
        print(f"mesh layout: {reason}", file=sys.stderr)
        return 0
    options.command = [command[0], *link]
    options.cc = False
    return run_link(options, stock_bytes=stock)


def record(args):
    require(sys.platform == "linux", "record requires Linux")
    require(1 <= args.runs <= 128 and args.timeout > 0, "invalid recording limits")
    binding = read_bounded_json(args.receipt)["stock"]
    elf_path = args.elf.resolve(strict=True)
    image = Elf(elf_path.read_bytes())
    require(sha256(image.data).hexdigest() == binding["elf_sha256"], "recording ELF digest mismatch")
    require(sha256(args.map.read_bytes()).hexdigest() == binding["map_sha256"], "recording map digest mismatch")
    match_map(image, args.map)
    page_size = os.sysconf("SC_PAGE_SIZE")
    counts = Counter()
    with tempfile.TemporaryDirectory(prefix="mesh-section-observe-", dir=args.output.resolve().parent) as directory:
        directory = Path(directory)
        if args.observer:
            observer = executable(args.observer)
        else:
            observer = directory / "observe"
            source = ENTRY.parent / "lib/mesh_section_observe.c"
            subprocess.run([executable(args.cc_driver), "-O2", "-Wall", "-Wextra", "-Werror",
                            source, "-o", observer], check=True)
        sample = directory / "pages"
        for _ in range(args.runs):
            sample.unlink(missing_ok=True)
            subprocess.run([observer, sample, elf_path, *after_separator(args.args)], check=True,
                           timeout=args.timeout)
            require(sample.stat().st_size <= 1024 * 1024, "page report exceeds 1 MiB")
            offsets = {int(line) for line in sample.read_text().splitlines()}
            require(offsets and all(0 <= offset < len(image.data) and offset % page_size == 0
                                    for offset in offsets), "invalid observed file pages")
            counts.update(offsets)
    require(sha256(elf_path.read_bytes()).hexdigest() == binding["elf_sha256"]
            and sha256(args.map.read_bytes()).hexdigest() == binding["map_sha256"],
            "recording ELF or map changed during capture")
    ranges = [[offset, min(page_size, len(image.data) - offset), count] for offset, count in sorted(counts.items())]
    write_json(args.output, {"schema": 1, **binding, "observations": args.runs,
                             "source": "exit-file-pte", "file_ranges": ranges})
    return 0


def train(args):
    require(0 <= args.text_bytes <= 16 * 1024 * 1024 and 0 <= args.rodata_bytes <= 16 * 1024 * 1024,
            "section byte budgets must be between zero and 16 MiB")
    profile, report = observed_profile(args.elf, args.map, args.runtime.resolve(),
                                       read_bounded_json(args.observations),
                                       args.text_bytes, args.rodata_bytes)
    write_json(args.output, profile)
    if args.report:
        write_json(args.report, report)
    return 0


def main():
    if Path(sys.argv[0]).name == "cc" and DRIVER_CONFIG in os.environ:
        return driver(sys.argv[1:])

    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    compiling = modes.add_parser("build", help="invoke the normal Mesh package build through its cc driver")
    compiling.add_argument("--meshc", default="meshc")
    compiling.add_argument("--runtime", required=True, type=Path)
    compiling.add_argument("--cc-driver", default="cc")
    compiling.add_argument("--cc-arg", action="append", default=[])
    compiling.add_argument("--cc-after", action="append", default=[])
    compiling.add_argument("--zig", action="store_true", help="capture LLD argv from a Zig cc driver")
    compiling.add_argument("--profile", type=Path)
    compiling.add_argument("--receipt", type=Path)
    compiling.add_argument("args", nargs=argparse.REMAINDER)
    compiling.set_defaults(run=build)

    recording = modes.add_parser("record", help="run the stock image explicitly and collect exit PTEs")
    recording.add_argument("--elf", type=Path, required=True)
    recording.add_argument("--map", type=Path, required=True)
    recording.add_argument("--receipt", type=Path, required=True)
    recording.add_argument("--output", type=Path, required=True)
    recording.add_argument("--runs", type=int, default=8)
    recording.add_argument("--timeout", type=float, default=30)
    recording.add_argument("--cc-driver", default="cc")
    recording.add_argument("--observer", help="use a previously built mesh_section_observe.c")
    recording.add_argument("args", nargs=argparse.REMAINDER)
    recording.set_defaults(run=record)

    training = modes.add_parser("train", help="map observed file offsets to relocatable runtime input identities")
    training.add_argument("--elf", type=Path, required=True)
    training.add_argument("--map", type=Path, required=True)
    training.add_argument("--runtime", type=Path, required=True)
    training.add_argument("--observations", type=Path, required=True)
    training.add_argument("--output", type=Path, required=True)
    training.add_argument("--report", type=Path)
    training.add_argument("--text-bytes", type=int, default=16 * 1024 * 1024)
    training.add_argument("--rodata-bytes", type=int, default=16 * 1024 * 1024)
    training.set_defaults(run=train)

    linking = modes.add_parser("link", help="apply a profile to an existing LLD or compiler driver command")
    linking.add_argument("--runtime", type=Path, required=True)
    linking.add_argument("--profile", type=Path)
    linking.add_argument("--receipt", type=Path)
    linking.add_argument("--cc", action="store_true", help="pass linker flags through the cc driver")
    linking.add_argument("command", nargs=argparse.REMAINDER)
    linking.set_defaults(run=run_link)
    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (Unsupported, OSError, KeyError, ValueError, TypeError, struct.error, subprocess.SubprocessError) as error:
        print(f"mesh layout: {error}", file=sys.stderr)
        raise SystemExit(1) from error
