"""Train and apply an optional GNU AArch64 ELF executable section order for LLD."""
from bisect import bisect_right
from collections import Counter, defaultdict
from dataclasses import dataclass
from fractions import Fraction
from hashlib import sha256
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import tempfile


# Only literal tokens that LLD can represent without glob interpretation.
TOKEN = re.compile(r"[-A-Za-z0-9_./+@]+\Z")
MAP_LINE = re.compile(r"^\s*([a-f0-9]+)\s+[a-f0-9]+\s+([a-f0-9]+)\s+(\d+) ( *)(\S.*)$")
ARCHIVE_MEMBER = re.compile(r"(.+)\(([^()]*)\)\Z")
PROFILE_TARGET = "aarch64-linux-gnu-elf-lld"
KINDS = {".text": 6, ".rodata": 2}  # SHF_ALLOC|SHF_EXECINSTR and SHF_ALLOC
LOAD, INTERP, STACK, RELRO = 1, 3, 0x6474E551, 0x6474E552


class Unsupported(ValueError):
    pass


def require(condition, reason):
    if not condition:
        raise Unsupported(reason)


def cstring(data, offset):
    return data[offset:data.index(0, offset)].decode()


class Elf:
    def __init__(self, data):
        self.data = data
        require(data[:7] == b"\x7fELF\x02\x01\x01", "expected ELF64 little endian")
        self.kind, machine, _, self.entry, phoff, shoff, _, _, phsize, phnum, shsize, shnum, names = (
            struct.unpack_from("<HHIQQQIHHHHHH", data, 16))
        require(machine == 183 and shsize == 64 and shnum > names > 0, "expected AArch64 ELF section table")
        self.sections = [struct.unpack_from("<IIQQQQIIQQ", data, shoff + i * 64) for i in range(shnum)]
        strings = self.contents(self.sections[names])
        self.names = [cstring(strings, s[0]) for s in self.sections]
        self.named = dict(zip(self.names, self.sections))
        require(not phnum or phsize == 56, "unsupported ELF program header format")
        self.programs = [struct.unpack_from("<IIQQQQQQ", data, phoff + i * 56) for i in range(phnum)]

    def contents(self, section):
        return self.data[section[4]:section[4] + section[5]]

    def symbols(self, table):
        strings = self.contents(self.sections[table[6]])
        return [(cstring(strings, name), info, other, ndx, value, size)
                for name, info, other, ndx, value, size in
                struct.iter_unpack("<IBBHQQ", self.contents(table))]


@dataclass(frozen=True)
class Input:
    output: str
    origin: str
    name: str
    address: int
    size: int
    alignment: int

    @property
    def kind(self):
        return self.output.removesuffix(".mesh_profile")

    @property
    def end(self):
        return self.address + self.size


def match_map(elf, path):
    """Reject maps from a debug/relocation layout or a different final link."""
    wanted = {name: (row[3], row[5], row[8]) for name, row in elf.named.items()
              if row[5] and row[2] & 7 in (2, 6) and row[1] == 1}
    with Path(path).open() as stream:
        matches = {}
        for line in stream:
            row = MAP_LINE.fullmatch(line.rstrip("\n"))
            if row and not row[4] and row[5] in wanted:
                va, size, align, _, name = row.groups()
                require(name not in matches, "duplicate output in training link map")
                matches[name] = int(va, 16), int(size, 16), int(align)
    require({".text", ".interp"} <= wanted.keys(), "unsupported training image")
    require(matches == wanted, "training ELF and LLD map output extents differ")


def map_inputs(path, include_internal=False):
    with Path(path).open() as stream:
        header = next(stream, "")
        require(all(column in header for column in ("VMA", "LMA", "Out", "Symbol")),
                "expected LLD link map")
        output = ""
        for line in stream:
            match = MAP_LINE.fullmatch(line.rstrip("\n"))
            if not match:
                continue
            va, size, alignment, indent, label = match.groups()
            if not indent:
                output = label
            elif len(indent) == 8 and label.endswith(")") and ":(" in label:
                origin, name = label[:-1].rsplit(":(", 1)
                if int(size, 16) and (include_internal or not origin.startswith("<")):
                    yield Input(output, origin, name, int(va, 16), int(size, 16), int(alignment))


def runtime_member(origin, runtime):
    match = ARCHIVE_MEMBER.fullmatch(origin)
    if match and Path(match[1]).resolve() == runtime:
        return match[2]
    return None


def archive_members(path):
    data = Path(path).read_bytes()
    require(data.startswith(b"!<arch>\n"), "expected a regular ar runtime archive")
    at = 8
    long_names = b""
    while at < len(data):
        header = data[at:at + 60]
        require(header[-2:] == b"`\n", "malformed ar member header")
        size = int(header[48:58])
        start = at + 60
        at = start + size + size % 2
        name = header[:16].decode().rstrip()
        if name == "//":
            long_names = data[start:start + size]
        elif name.startswith("#1/"):
            length = int(name[3:])
            name = data[start:start + length].rstrip(b"\0").decode()
            yield name, data[start + length:start + size]
        elif name.startswith("/") and name[1:].isdigit():
            offset = int(name[1:])
            name = long_names[offset:long_names.index(b"/\n", offset)].decode()
            yield name, data[start:start + size]
        elif not name.startswith("/"):
            yield name.removesuffix("/"), data[start:start + size]


def eligible(runtime):
    members = defaultdict(list)
    duplicate_members = set()
    for name, blob in archive_members(runtime):
        if name in members:
            duplicate_members.add(name)
            continue
        members[name] = []
        if "/" in name or not TOKEN.fullmatch(name) or not blob.startswith(b"\x7fELF"):
            continue
        elf = Elf(blob)
        require(elf.kind == 1, "runtime archive has a non-relocatable ELF member")
        counts = Counter(elf.names)
        for section_name, section in zip(elf.names, elf.sections):
            kind = section_name.split(".", 2)
            kind = "." + kind[1] if len(kind) == 3 else ""
            if (kind in KINDS and section[1] == 1 and section[2] == KINDS[kind]
                    and counts[section_name] == 1 and "/" not in section_name and TOKEN.fullmatch(section_name)):
                members[name].append((section_name, section[5], section[8]))
    return {(name, sec): (size, align) for name, rows in members.items()
            if name not in duplicate_members for sec, size, align in rows}, duplicate_members


def live_runtime(rows, runtime):
    live = defaultdict(list)
    for row in rows:
        member = runtime_member(row.origin, runtime)
        if member and row.kind in KINDS and row.name.startswith(row.kind + "."):
            live[member, row.name].append(row)
    return live


def select(profile, rows, runtime):
    identity, duplicates = eligible(runtime)
    live = live_runtime(rows, runtime)
    counts = Counter()
    totals = Counter()
    selected = []
    seen = set()
    require(profile["schema"] == 1 and profile["target"] == PROFILE_TARGET, "unsupported profile")
    # A map address is never an identity. The runtime path binds the runtime
    # role for this link only and does not participate in the profile.
    for member, section, size, alignment in profile["sections"]:
        key = member, section
        require(key not in seen, f"duplicate profile entry {key!r}")
        seen.add(key)
        if member in duplicates or len(live[key]) > 1:
            reason = "ambiguous"
        elif key not in identity:
            reason = "missing_or_ineligible"
        elif identity[key] != (size, alignment):
            reason = "metadata_changed"
        elif not live[key]:
            reason = "not_live"
        elif live[key][0].size != size or live[key][0].alignment != alignment:
            reason = "linker_changed_section"
        else:
            reason = "matched"
            selected.append((member, live[key][0]))
        counts[reason] += 1
        totals[reason] += size
    return selected, {"sections": dict(counts), "trained_bytes": dict(totals),
                      "runtime_duplicate_members": len(duplicates)}


def fragment(selected):
    output = []
    for kind in KINDS:
        lines = []
        for member, row in selected:
            if row.kind != kind:
                continue
            archive, _ = ARCHIVE_MEMBER.fullmatch(row.origin).groups()
            require(TOKEN.fullmatch(archive), "runtime path contains characters unsafe in an LLD selector")
            lines.append(f"  {archive}:{member}({row.name})")
        if lines:
            output.append(f"SECTIONS {{ {kind}.mesh_profile : {{\n" + "\n".join(lines)
                          + f"\n}} }} INSERT BEFORE {kind};\n")
    return "".join(output)


def verify_elf(stock, candidate):
    for elf in (stock, candidate):
        require(elf.kind in (2, 3), "expected main executable")
        require([elf.contents(elf.named[".interp"])] == [b"/lib/ld-linux-aarch64.so.1\0"],
                "expected GNU AArch64 interpreter")
        require(any(p[0] == RELRO for p in elf.programs)
                and [p[1] for p in elf.programs if p[0] == STACK] == [6], "release protections missing")
        loads = [p for p in elf.programs if p[0] == LOAD]
        require(all(p[1] in (4, 5, 6) and p[7] == 65536 and p[2] % p[7] == p[3] % p[7]
                    and p[5] <= p[6] and p[2] + p[5] <= len(elf.data) for p in loads),
                "changed load segment geometry")
        require(any(p[1] == 5 and p[3] <= elf.entry < p[3] + p[5] for p in loads),
                "entry point is outside executable load segment")
        for kind, flags in ((".text", 5), (".rodata", 4)):
            section = elf.named.get(kind + ".mesh_profile")
            if section is not None:
                require(section[1:3] == (1, KINDS[kind]) and
                        any(p[1] == flags and p[3] <= section[3] and
                            section[3] + section[5] <= p[3] + p[5] for p in loads),
                        f"profile section escaped its original permissions: {kind}")
    require(stock.kind == candidate.kind and [(p[0], p[1], p[7]) for p in stock.programs]
            == [(p[0], p[1], p[7]) for p in candidate.programs], "program header policy changed")
    # Compare dynamic ABI and protection tags while allowing addresses to move.
    pointer_tags = {3, 4, 5, 6, 7, 12, 13, 17, 23, 25, 26, 32, 34, 35, 36,
                    0x6FFFFEF5, 0x6FFFFFF0, 0x6FFFFFFC, 0x6FFFFFFE}
    def abi(elf):
        table = list(struct.iter_unpack("<QQ", elf.contents(elf.named[".dynamic"])))
        require(all(tag != 22 for tag, _ in table) and
                any((tag == 30 and value & 8) or (tag == 0x6FFFFFFB and value & 1)
                    for tag, value in table), "text relocations or lazy binding")
        syms = [(name, info, other, elf.names[ndx].removesuffix(".mesh_profile")
                 if 0 < ndx < len(elf.names) else ndx, size)
                for name, info, other, ndx, _, size in elf.symbols(elf.named[".dynsym"])]
        fixed = [elf.contents(elf.named[name]) for name in (".dynstr", ".gnu.version", ".gnu.version_r")
                 if name in elf.named]
        return [(tag, 0 if tag in pointer_tags else value) for tag, value in table], syms, fixed
    require(abi(stock) == abi(candidate), "dynamic ABI changed")
    require([(p[0], p[5:7]) for p in stock.programs if p[0] in (7, STACK)] ==
            [(p[0], p[5:7]) for p in candidate.programs if p[0] in (7, STACK)],
            "TLS or stack size changed")


def verify_placement(stock, candidate, selected):
    def identity(row):
        return row.kind, row.origin, row.name, row.size, row.alignment

    require(Counter(map(identity, stock)) == Counter(map(identity, candidate)),
            "retained input inventory changed")
    for kind in (".preinit_array", ".init_array", ".fini_array"):
        require([identity(row) for row in stock if row.output == kind] ==
                [identity(row) for row in candidate if row.output == kind],
                f"{kind} order changed")
    for kind in KINDS:
        require([identity(row) for _, row in selected if row.kind == kind] ==
                [identity(row) for row in candidate if row.output == kind + ".mesh_profile"],
                f"{kind} placements did not match the generated fragment")


def run_link(args, stock_bytes=None):
    command = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        raise SystemExit("expected a compiler driver or LLD command after --")
    runtime = args.runtime.resolve()
    if not args.profile and not args.receipt:
        return 0 if stock_bytes is not None else subprocess.call(command)

    def fallback(reason):
        status = 0 if stock_bytes is not None else subprocess.call(command)
        if args.receipt and not status:
            write_json(args.receipt, {"state": "stock", "reason": reason})
        print(f"mesh layout: {reason}", file=os.sys.stderr)
        return status

    if args.cc:
        linker_args = [item for arg in command if arg.startswith("-Wl,") for item in arg[4:].split(",")]
    else:
        linker_args = command[1:]
    plain = [arg for arg in command[1:] if not arg.startswith("-")]
    safe = (command.count("-o") == 1 and "--gc-sections" in linker_args
            and sum(Path(arg).resolve() == runtime for arg in plain) == 1
            and not any(arg.startswith(("@", "-T", "-Map", "--script", "--icf", "--symbol-ordering",
                                        "--section-ordering", "--oformat", "--defsym", "--section-start"))
                        or arg in ("-shared", "--shared", "-r", "--relocatable", "-c", "-Xlinker")
                        for arg in (*command[1:], *linker_args))
            and "--no-gc-sections" not in linker_args
            and not any(arg.startswith(("-o", "-dT", "--default-script")) and arg != "-o"
                        for arg in linker_args))
    if not safe:
        return fallback("unsupported command, using stock link")
    position = command.index("-o") + 1
    if command[position] == "-":
        return subprocess.call(command)
    output = Path(os.path.abspath(command[position]))
    inputs = {path for arg in plain if (path := Path(arg).resolve()).is_file() and path != output.resolve()}
    def stamps():
        result = {}
        for path in inputs:
            info = path.stat()
            result[path] = info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns
        return result

    baseline_stamps = stamps()
    receipt = {"state": "stock"}
    prefix = "-Wl," if args.cc else ""
    with tempfile.TemporaryDirectory(prefix=".mesh-layout-", dir=output.parent) as temp:
        work = Path(temp)
        stock_map, new_map = work / "stock.map", work / "candidate.map"
        if args.cc and "," in str(stock_map):
            return fallback("compiler driver cannot forward this map path")
        stock_mode = output.stat().st_mode if stock_bytes is not None else None
        mapped = subprocess.run([*command, f"{prefix}-Map={stock_map}"], capture_output=True)
        if stock_bytes is not None and (mapped.returncode or not output.is_file()
                                        or output.read_bytes() != stock_bytes):
            preserved = work / "original"
            preserved.write_bytes(stock_bytes)
            preserved.chmod(stock_mode)
            os.replace(preserved, output)
            return fallback("stock driver and direct map relink differ")
        if mapped.returncode:
            return fallback("linker could not emit a map, using stock link")
        if mapped.stderr:
            os.sys.stderr.buffer.write(mapped.stderr)
        if mapped.stdout:
            os.sys.stdout.buffer.write(mapped.stdout)
        try:
            baseline = Elf(output.read_bytes())
            match_map(baseline, stock_map)
            receipt["stock"] = {"elf_sha256": sha256(baseline.data).hexdigest(),
                                "map_sha256": sha256(stock_map.read_bytes()).hexdigest()}
            if not args.profile:
                return 0
            profile = read_bounded_json(args.profile)
            rows = list(map_inputs(stock_map))
            selected, receipt["coverage"] = select(profile, rows, runtime)
            require(selected, "no unambiguous live profile entries")
            script = work / "order.ld"
            script.write_text(fragment(selected))
            candidate = work / "candidate"
            relink = command.copy()
            relink[position] = str(candidate)
            relink += [f"{prefix}-T{script}", f"{prefix}-Map={new_map}"]
            proc = subprocess.run(relink, capture_output=True, text=True)
            require(proc.returncode == 0 and not proc.stderr.strip(),
                    f"candidate linker status {proc.returncode}: {proc.stderr.strip()}")
            verify_placement(rows, list(map_inputs(new_map)), selected)
            chosen = Elf(candidate.read_bytes())
            verify_elf(baseline, chosen)
            require(stamps() == baseline_stamps, "link inputs changed during profiling")
            receipt["candidate_elf_sha256"] = sha256(chosen.data).hexdigest()
            if args.receipt:
                for path in (script, new_map):
                    args.receipt.with_suffix("." + path.name).write_bytes(path.read_bytes())
            os.replace(candidate, output)
            receipt["state"] = "candidate"
        except (ValueError, OSError, KeyError, TypeError, IndexError, struct.error) as error:
            receipt["reason"] = str(error)
        finally:
            if args.receipt:
                if "stock" in receipt:
                    args.receipt.with_suffix(".stock.map").write_bytes(stock_map.read_bytes())
                write_json(args.receipt, receipt)
    if "reason" in receipt:
        print(f"mesh layout: {receipt['reason']}. Retained stock executable.", file=os.sys.stderr)
    return 0


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def read_bounded_json(path):
    with Path(path).open("rb") as stream:
        data = stream.read(16 * 1024 * 1024 + 1)
    require(len(data) <= 16 * 1024 * 1024, "JSON input exceeds 16 MiB")
    return json.loads(data)


def observed_ranges(elf, map_path, observations):
    require(observations["schema"] == 1, "unsupported observation schema")
    require(observations["elf_sha256"] == sha256(elf.data).hexdigest(), "observation ELF digest mismatch")
    require(observations["map_sha256"] == sha256(map_path.read_bytes()).hexdigest(),
            "observation map digest mismatch")
    match_map(elf, map_path)
    instances = observations["observations"]
    require(type(instances) is int and 1 <= instances <= 1_000_000, "invalid observation count")
    ranges = observations["file_ranges"]
    require(isinstance(ranges, list) and 1 <= len(ranges) <= 65_536, "invalid file-range count")
    previous_end = 0
    for row in ranges:
        require(isinstance(row, list) and len(row) == 3 and all(type(n) is int for n in row),
                "expected file offset, length and frequency")
        offset, length, count = row
        require(offset >= previous_end and 1 <= length <= len(elf.data) - offset and 1 <= count <= instances,
                "unsorted, overlapping or out-of-bounds file range")
        previous_end = offset + length
    return ranges


def observed_profile(elf_path, map_path, archive, observations, text_limit, rodata_limit):
    """Rank observed runtime inputs by frequency-weighted coverage per moved byte."""
    elf = Elf(elf_path.read_bytes())
    require(elf.kind in (2, 3), "expected a linked executable")
    ranges = observed_ranges(elf, map_path, observations)
    ends = [offset + length for offset, length, _ in ranges]
    rows = list(map_inputs(map_path, include_internal=True))
    allowed, duplicate_members = eligible(archive)
    live = live_runtime(rows, archive)
    counts = defaultdict(Counter)
    excluded_objects = Counter()
    ranked = defaultdict(list)
    for row in rows:
        if row.output not in KINDS:
            continue
        section = elf.named[row.output]
        require(section[3] <= row.address and row.end <= section[3] + section[5],
                "map contribution lies outside its linked section")
        low = section[4] + row.address - section[3]
        high = low + row.size
        unique, weighted = 0, 0
        i = bisect_right(ends, low)
        while i < len(ranges) and ranges[i][0] < high:
            start, length, frequency = ranges[i]
            size = min(start + length, high) - max(start, low)
            unique += size
            weighted += size * frequency
            i += 1
        if not weighted:
            continue
        member = runtime_member(row.origin, archive)
        key = member, row.name
        if member is None:
            if row.origin.startswith("<"):
                category = "linker_generated"
            elif ARCHIVE_MEMBER.fullmatch(row.origin):
                category = "other_archive"
            else:
                category = "other_object"
                excluded_objects[row.origin] += unique
        elif (allowed.get(key) != (row.size, row.alignment) or len(live[key]) != 1
              or member in duplicate_members):
            category = "ineligible_runtime"
        else:
            category = "eligible_runtime"
            ranked[row.kind].append((Fraction(weighted, row.size), weighted, unique, member, row))
        stats = counts[row.kind]
        stats[f"{category}_sections"] += 1
        stats[f"{category}_observed_bytes"] += unique
        stats[f"{category}_weighted_bytes"] += weighted
    output = []
    for kind, limit in ((".text", text_limit), (".rodata", rodata_limit)):
        remaining = limit
        for _, weighted, unique, member, row in sorted(ranked[kind],
                key=lambda entry: (-entry[0], -entry[1], entry[3], entry[4].name)):
            if row.size <= remaining:
                output.append([member, row.name, row.size, row.alignment])
                remaining -= row.size
                counts[kind]["selected_observed_bytes"] += unique
                counts[kind]["selected_weighted_bytes"] += weighted
        counts[kind]["selected_input_bytes"] = limit - remaining
    require(output, "observations did not include eligible runtime inputs")
    profile = {"schema": 1, "target": PROFILE_TARGET, "sections": output}
    total = sum(row[1] for row in ranges)
    attributed = sum(value for counter in counts.values() for name, value in counter.items()
                     if name.endswith("_observed_bytes") and not name.startswith("selected_"))
    require(attributed <= total, "map contributions overlap within observed file ranges")
    report = {"observations": observations["observations"], "file_ranges": len(ranges),
              "observed_file_bytes": total, "unattributed_file_bytes": total - attributed,
              "weighted_file_bytes": sum(row[1] * row[2] for row in ranges),
              "runtime_duplicate_members": len(duplicate_members), "coverage": counts,
              "other_objects_observed_bytes": dict(excluded_objects.most_common())}
    require(sha256(map_path.read_bytes()).hexdigest() == observations["map_sha256"],
            "training map changed during import")
    return profile, report
