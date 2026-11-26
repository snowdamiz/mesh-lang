"""Rebuild the frozen Jobs GNU AArch64 ELF with optional exact section ordering."""

import argparse
from collections import Counter, defaultdict
from hashlib import file_digest, sha256
import json
import os
from pathlib import Path
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT / "scripts/lib"))
from mesh_section_layout import ARCHIVE_MEMBER, Elf, map_inputs, match_map, require

SECTIONS = {".rodata": "BEFORE", ".text": "AFTER"}


def digest(path):
    with path.open("rb") as stream:
        return file_digest(stream, "sha256").hexdigest()


def read(path):
    return json.loads(path.read_text())


def check_hash(actual, expected, name):
    require(actual == expected, f"{name} SHA-256 {actual} differs from {expected}")


def origin_name(origin):
    match = ARCHIVE_MEMBER.fullmatch(origin)
    if match:
        return f"{Path(match[1]).name}({match[2]})"
    return Path(origin).name


def order_rows(path, sections):
    rows = defaultdict(list)
    for row in map_inputs(path):
        if row.output in sections:
            rows[row.output].append([origin_name(row.origin), row.name, row.address,
                                     row.size, row.alignment])
    return dict(rows)


def order_digest(rows):
    return sha256(json.dumps(rows, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def layout_script(stock, indices):
    script = []
    expected = {}
    for section, position in SECTIONS.items():
        order = indices[section]
        require(order and len(order) == len(set(order)), f"{section} contains duplicate indices")
        rows = stock[section]
        require(all(0 <= i < len(rows) for i in order), f"{section} index out of range")
        selected = [rows[i] for i in order]
        counts = Counter((row[0], row[1]) for row in rows)
        selectors = []
        for origin, name, _, _, _ in selected:
            require(counts[origin, name] == 1, f"ambiguous input section {origin}:{name}")
            archive, sep, member = origin.partition("(")
            qualifier = f"{archive}:{member.removesuffix(')')}" if sep else archive
            selectors.append(f"  {qualifier}({name})\n")
        script.append(f"SECTIONS {{ {section}.profile : {{\n")
        script.extend(selectors)
        script.append(f"}} }} INSERT {position} {section};\n")
        expected[section + ".profile"] = [row[:2] + row[3:] for row in selected]
    return "".join(script), expected


def relink(root, output, runtime, driver, layout):
    import fcntl

    manifest = read(HERE / "link-inputs.json")
    require(manifest["schema"] == 1, "unsupported link manifest")
    inputs = manifest["inputs"]
    paths = {name: (root / entry["path"]).resolve(strict=True)
             for name, entry in inputs.items() if name not in ("runtime-candidate.a", "zig")}
    paths.update({"runtime-candidate.a": runtime.resolve(strict=True),
                  "zig": driver.resolve(strict=True)})
    for name, path in paths.items():
        check_hash(digest(path), inputs[name]["sha256"], name)
    output.mkdir()
    for name, path in paths.items():
        if name != "zig":
            (output / name).symlink_to(path)
    for folder in ("tmp", "zig-global", "zig-local"):
        (output / folder).mkdir()
    env = dict(os.environ, TMPDIR=str(output / "tmp"),
               ZIG_GLOBAL_CACHE_DIR=str(output / "zig-global"),
               ZIG_LOCAL_CACHE_DIR=str(output / "zig-local"))
    argv = [str(paths["zig"]), *manifest["lld_argv"]]

    def link(filename, mapname, *, symbols=False, profiled=False):
        flags = ["--strip-debug"] if symbols else ["-s"]
        if profiled:
            flags += ["-T", "order.ld"]
        command = argv + ["-o", filename, f"-Map={mapname}", *flags]
        subprocess.run(command, cwd=output, env=env, check=True)
        match_map(Elf((output / filename).read_bytes()), output / mapname)
        return digest(output / filename)

    (ROOT / "target").mkdir(exist_ok=True)
    with (ROOT / "target/queue-performance.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        stock_sha = link("mesh-stock", "stock.ship.map")
        check_hash(stock_sha, manifest["unarranged_elf_sha256"], "unarranged ELF")
        result = {"unarranged_elf_sha256": stock_sha}
        if layout:
            symbol_sha = link("mesh-stock-symbols", "stock.link.map", symbols=True)
            check_hash(symbol_sha, manifest["unarranged_symbols_sha256"], "unarranged symbols ELF")
            stock = order_rows(output / "stock.link.map", SECTIONS)
            check_hash(order_digest(stock), manifest["unarranged_order_sha256"], "unarranged input order")
            indices = read(HERE / "section-order.json")
            script, expected = layout_script(stock, indices)
            check_hash(sha256(script.encode("ascii")).hexdigest(),
                       manifest["portable_linker_script_sha256"], "generated linker script")
            (output / "order.ld").write_text(script, encoding="ascii")
            candidate_sha = link("mesh", "layout.ship.map", profiled=True)
            check_hash(candidate_sha, manifest["layout_elf_sha256"], "layout ELF")
            placed = order_rows(output / "layout.ship.map", expected)
            observed = {section: [row[:2] + row[3:] for row in rows]
                        for section, rows in placed.items()}
            require(observed == expected, "LLD placed different input sections")
            check_hash(order_digest(placed), manifest["placed_order_sha256"], "actual placed input order")
            result.update(layout_elf_sha256=candidate_sha, actual_order_sha256=order_digest(placed),
                          placed_sections={name: len(rows) for name, rows in placed.items()})
        for name, path in paths.items():
            check_hash(digest(path), inputs[name]["sha256"], name)
    (output / "verification.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="fresh output directory")
    parser.add_argument("--input-root", type=Path, default=ROOT)
    parser.add_argument("--runtime", type=Path, help="override the runtime archive path")
    parser.add_argument("--zig", type=Path, help="override the pinned Zig driver path")
    parser.add_argument("--layout", action="store_true", help="also link and verify the profiled ELF")
    args = parser.parse_args()
    inputs = read(HERE / "link-inputs.json")["inputs"]
    relink(args.input_root, args.output.resolve(),
           args.runtime or args.input_root / inputs["runtime-candidate.a"]["path"],
           args.zig or args.input_root / inputs["zig"]["path"], args.layout)


if __name__ == "__main__":
    main()
