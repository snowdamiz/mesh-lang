"""Extract and relink the exact archived Jobs ordering, using pinned inputs."""

import argparse
from hashlib import sha256
import json
import os
from pathlib import Path, PurePosixPath
import re
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
MAP_ROW = re.compile(r"^\s*[a-f0-9]+\s+[a-f0-9]+\s+([a-f0-9]+)\s+\d+ ( *)(.+)$")
CRT_MARKER = b"target/dense-cpu-sampling/zig-global/o/b22038b94896a40f7138eaa0cc50b274/crt1.o"


def digest(data):
    return sha256(data).hexdigest()


def file_digest(path):
    with path.open("rb") as stream:
        checksum = sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(block)
    return checksum.hexdigest()


def require_digest(actual, expected, artifact):
    if actual != expected:
        raise ValueError(f"{artifact} SHA-256 differs: {actual} != {expected}")


def normalize_map(data):
    """Remove only the captured checkout prefix shared by the input paths."""
    line = next(row for row in data.splitlines() if CRT_MARKER + b":(" in row)
    row = MAP_ROW.fullmatch(line.decode())
    assert row and len(row[2]) == 8
    origin = row[3].split(":(", 1)[0].encode()
    assert origin.endswith(CRT_MARKER)
    prefix = origin[:-len(CRT_MARKER)]
    assert not prefix or prefix.startswith(b"/") and prefix.endswith(b"/")
    return data.replace(prefix, b"") if prefix else data


def selector(output, origin, section):
    assert origin != "<internal>" and re.fullmatch(r"\.[A-Za-z0-9_.$]+", section)
    if output == ".rodata":
        assert section.startswith(".rodata.")
        return f"  *({section})\n"
    assert output == ".text" and (section == ".text" or section.startswith(".text."))
    if origin.endswith(")"):
        archive, member = origin[:-1].rsplit("(", 1)
        qualifier = f"*{PurePosixPath(archive).name}:*{PurePosixPath(member).name}"
    else:
        qualifier = f"*{PurePosixPath(origin).name}"
    assert re.fullmatch(r"[A-Za-z0-9.*_:-]+", qualifier)
    return f"  {qualifier}({section})\n"


def contributions(normalized):
    output = None
    results = {}
    for number, line in enumerate(normalized.decode().splitlines(), start=1):
        match = MAP_ROW.fullmatch(line)
        if not match:
            continue
        size, indent, name = match.groups()
        if not indent:
            output = name
        elif len(indent) == 8 and int(size, 16) and output in (".rodata", ".text"):
            assert name.endswith(")") and ":(" in name
            origin, section = name[:-1].rsplit(":(", 1)
            if origin != "<internal>" and (output == ".text" or section.startswith(".rodata.")):
                results[number] = (output, selector(output, origin, section))
    return results


def extract(data, indices, provenance):
    archived = provenance["input_and_proof_artifacts_sha256"]
    assert indices["stock_map_sha256"] == archived["stock/link.map"]
    assert indices["linker_script_sha256"] == archived["exact-inputs/order.ld"]
    normalized = normalize_map(data)
    require_digest(digest(normalized), indices["normalized_stock_map_sha256"], "normalized stock map")
    rows = contributions(normalized)
    result = []
    for output, count in ((".rodata", 1666), (".text", 2228)):
        numbers = indices["map_line_numbers"][output]
        assert len(numbers) == len(set(numbers)) == count
        selected = [rows[index] for index in numbers]
        assert all(name == output for name, _ in selected)
        assert len({value for _, value in selected}) == count
        result.append(f"SECTIONS {{ {output}.profile : {{\n")
        result.extend(value for _, value in selected)
        result.append(f"}} }} INSERT BEFORE {output};\n")
    script = "".join(result).encode("ascii")
    require_digest(digest(script), indices["linker_script_sha256"], "reconstructed linker script")
    return script


def run_link(argv, root, inputs, output, env, *, map_path=None, script=None):
    command = [str(root / token) if token in inputs else token for token in argv]
    command[command.index("-o") + 1] = str(output)
    if script:
        command[command.index("-T") + 1] = str(script)
    if map_path:
        command.remove("-s")
        command += ["--strip-debug", f"-Map={map_path}"]
    subprocess.run(command, cwd=root, env=env, check=True)
    return command


def relink(root, out, supplied_map, indices, provenance):
    import fcntl

    out.mkdir(exist_ok=False)
    archived = provenance["input_and_proof_artifacts_sha256"]
    inputs = provenance["retained_link_input_sha256"]
    argv = provenance["release_link_argv"]
    selection_path = argv["exact-inputs"][argv["exact-inputs"].index("-T") + 1]
    assert inputs[selection_path] == indices["linker_script_sha256"]
    assert set(inputs) == {arg for args in argv.values() for arg in args
                           if arg.startswith("target/") and arg not in {
                               args[args.index("-o") + 1]}}
    for name, wanted in inputs.items():
        path = PurePosixPath(name)
        assert not path.is_absolute() and ".." not in path.parts
        if name != selection_path:
            require_digest(file_digest(root / name), wanted, name)

    commands = {}
    with (root / "target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        with tempfile.TemporaryDirectory(dir=out, prefix="link-cache-") as tmp:
            cache = Path(tmp)
            for name in ("global", "local"):
                (cache / name).mkdir()
            env = dict(os.environ, TMPDIR=str(cache), ZIG_GLOBAL_CACHE_DIR=str(cache / "global"),
                       ZIG_LOCAL_CACHE_DIR=str(cache / "local"))
            stock = out / "mesh-stock"
            commands["stock"] = run_link(argv["stock"], root, inputs, stock, env)
            require_digest(file_digest(stock), archived["stock/mesh"], "reconstructed stock ELF")
            if supplied_map is None:
                supplied_map = out / "stock.link.map"
                commands["stock_map"] = run_link(argv["stock"], root, inputs, cache / "stock-symbols", env,
                                                 map_path=supplied_map)
            script = out / "order.ld"
            script.write_bytes(extract(supplied_map.read_bytes(), indices, provenance))
            candidate = out / "mesh"
            commands["candidate"] = run_link(argv["exact-inputs"], root, inputs, candidate, env, script=script)
            require_digest(file_digest(candidate), archived["exact-inputs/mesh"], "reconstructed candidate ELF")
    (out / "verification.json").write_text(json.dumps({
        "map_sha256": file_digest(supplied_map), "normalized_map_sha256": indices["normalized_stock_map_sha256"],
        "order_sha256": file_digest(script), "stock_sha256": file_digest(stock), "candidate_sha256": file_digest(candidate),
        "link_commands": commands,
    }, indent=2) + "\n")
    print(f"{file_digest(candidate)}  {candidate.name}")


def main():
    if not __debug__:
        raise RuntimeError("Run without Python optimization to enable verification")
    cli = argparse.ArgumentParser(description=__doc__)
    commands = cli.add_subparsers(dest="command", required=True)
    unpack = commands.add_parser("extract")
    unpack.add_argument("stock_map", type=Path)
    unpack.add_argument("linker_script", type=Path)
    build = commands.add_parser("relink")
    build.add_argument("pinned_input_root", type=Path)
    build.add_argument("output_directory", type=Path)
    build.add_argument("--stock-map", type=Path,
                       help="use an existing pinned map instead of recreating the symbol sidecar")
    options = cli.parse_args()
    provenance = json.loads((HERE / "link-provenance.json").read_text())
    indices = json.loads((HERE / "order-map-indices.json").read_text())
    if options.command == "extract":
        script = extract(options.stock_map.read_bytes(), indices, provenance)
        with options.linker_script.open("xb") as stream:
            stream.write(script)
        print(f"{digest(script)}  {options.linker_script.name}")
    else:
        relink(options.pinned_input_root.resolve(), options.output_directory.resolve(),
               options.stock_map, indices, provenance)


if __name__ == "__main__":
    main()
