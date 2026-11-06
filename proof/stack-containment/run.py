#!/usr/bin/env python3
"""Bounded LLVM 21 experiments. Outputs stay in the assigned checkout."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "proof/stack-containment"
OUT = ROOT / ".tmp/stack-containment/run"
OUT.mkdir(parents=True, exist_ok=True)
(OUT / "results.json").unlink(missing_ok=True)
if (platform.system(), platform.machine()) != ("Darwin", "arm64"):
    raise SystemExit(f"FAIL: unsupported native host {platform.system()} {platform.machine()}; "
                     "run on Darwin arm64. Linux and Windows targets are codegen-only.")

import resource


def require(condition, message):
    if not condition:
        raise SystemExit(f"FAIL: {message}")


def tool(command, hint):
    path = shutil.which(str(command)) if command else None
    require(path is not None, f"required executable {command!s} not found or not executable; {hint}")
    return str(Path(path).absolute())


prefix = os.environ.get("LLVM_SYS_211_PREFIX")
LLVM = {name: tool(Path(prefix) / "bin" / name if prefix is not None else name,
                  "set LLVM_SYS_211_PREFIX to an LLVM 21 installation or put its bin directory on PATH")
        for name in ("llc", "opt", "clang", "clang++", "llvm-config")}
RUSTC = tool(os.environ.get("RUSTC", "rustc"), "set RUSTC to the Rust compiler executable or put rustc on PATH")
GIT = tool("git", "install git and put it on PATH")
TARGETS = {
    "darwin-arm64": "arm64-apple-darwin",
    "linux-arm64": "aarch64-unknown-linux-gnu",
    "linux-x86_64": "x86_64-unknown-linux-gnu",
    "windows-x86_64": "x86_64-pc-windows-msvc",
}
records = []


def limits():
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    resource.setrlimit(resource.RLIMIT_CPU, (30, 30))


def run(name, command, expected=(0,)):
    command = [str(v) for v in command]
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True,
                            timeout=45, preexec_fn=limits)
    entry = dict(name=name, command=shlex.join(command), rc=result.returncode,
                 stdout=result.stdout, stderr=result.stderr)
    records.append(entry)
    (OUT / f"{name}.log").write_text(json.dumps(entry, indent=2) + "\n")
    if result.returncode not in expected:
        raise RuntimeError(f"{name}: exit {result.returncode}\n{result.stderr}")
    return result


def frame_sizes(stderr):
    return {name: int(size) for size, name in re.findall(
        r"stack frame size \((\d+)\).*?in function '([^']+)'", stderr)}


def functions(assembly):
    return {name: body for name, body in re.findall(
        r"^_?(\w+):[^\n]*\n(.*?)(?=^\s*[;#] -- End function|^\s*\.cfi_endproc|^\s*\.seh_endproc)",
        assembly, re.M | re.S)}


def check_diagnostics(name, result, mode, expected_frames, cap):
    rows = re.findall(r"^frame=(\w+) bytes=(\d+) severity=(\d+)$", result.stdout, re.M)
    frames = {function: int(size) for function, size, _ in rows}
    require(set(frames) == expected_frames and len(rows) == len(frames),
            f"{name}: expected frame diagnostics for {sorted(expected_frames)}, got {frames}")
    require(all(int(size) > cap and severity == "1" for _, size, severity in rows),
            f"{name}: frame warning must exceed the inclusive cap {cap}")
    fields = {key: int(value) for key, value in re.findall(r"\b(\w+)=(\d+)\b", result.stdout)
              if key not in {"bytes", "severity"}}
    count = len(frames)
    promoted = mode.endswith("-error")
    expected = dict(stack_diagnostics=count, error_diagnostics=count if promoted else 0,
                    context_has_errors=int(promoted and count > 0), rejected=int(count > 0))
    if mode.startswith("jit"):
        expected.update(create_rc=0, diagnostics_before_lookup=0, address_nonzero=1,
                        diagnostics_after_lookup=count, executed=0)
    else:
        expected.update(emit_rc=0, published=0)
        require(fields.get("output_bytes", 0) > 0, f"{name}: object emission must produce a private buffer")
    require(all(fields.get(key) == value for key, value in expected.items()),
            f"{name}: diagnostic/publication outcome differs; expected {expected}, got {fields}")


llvm_version = run("llvm-config-version", [LLVM["llvm-config"], "--version"]).stdout.strip()
require(llvm_version.startswith("21.1."),
        f"LLVM 21.1 is required, got {llvm_version}; set LLVM_SYS_211_PREFIX to LLVM 21.1")
rust_version = run("rust-version", [RUSTC, "-vV"]).stdout.strip()
require("host: aarch64-apple-darwin" in rust_version,
        "RUSTC must be a native aarch64-apple-darwin compiler for the pthread unwind probe")


base = (SRC / "frames.ll").read_text()
outgoing = "declare void @many_args(" + ", ".join(["i64"] * 800) + ")\n"
outgoing += "define i64 @outgoing() #0 {\n  call void @many_args("
outgoing += ", ".join(["i64 7"] * 800)
outgoing += ")\n  %sp = call ptr @llvm.stacksave.p0()\n  %n = ptrtoint ptr %sp to i64\n  ret i64 %n\n}\n"
base = base.replace("; OUTGOING", outgoing)
inputs = {}
for mode in ("none", "inline", "strict"):
    source = base.replace('"warn-stack-size"="4096"', '"warn-stack-size"="0"')
    if mode == "none":
        source = source.replace('"probe-stack"="inline-asm"', "")
    if mode == "strict":
        source = source.replace("noinline uwtable", "noinline uwtable noredzone")
    path = OUT / f"{mode}.ll"
    path.write_text(source)
    inputs[mode] = path

metadata = {
    "base_commit": run("head", [GIT, "rev-parse", "HEAD"]).stdout.strip(),
    "host": platform.platform(),
    "native_runtime_target": "darwin-arm64",
    "host_page_size": os.sysconf("SC_PAGE_SIZE"),
    "llvm": run("llvm-version", [LLVM["llc"], "--version"]).stdout,
    "rustc": rust_version,
    "source_sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted(SRC.iterdir()) if p.suffix in {".py", ".ll", ".rs", ".c", ".cpp"}},
}
matrix = []
for target, triple in TARGETS.items():
    for level in (0, 2):
        for mode, source in inputs.items():
            name = f"{target}-O{level}-{mode}"
            ir = source
            if level == 2:
                ir = OUT / f"{name}.ll"
                run(name + "-opt", [LLVM["opt"], "-passes=default<O2>",
                    f"-mtriple={triple}", "-S", source, "-o", ir])
            assembly = OUT / f"{name}.s"
            result = run(name, [LLVM["llc"], f"-O{level}", f"-mtriple={triple}",
                         "-filetype=asm", ir, "-o", assembly])
            bodies = functions(assembly.read_text())
            matrix.append(dict(target=target, optimization=f"O{level}", mode=mode,
                frames=frame_sizes(result.stderr),
                outgoing_pushes=len(re.findall(r"\bpushq\b", bodies.get("outgoing", ""))),
                leaf_probe_instructions=[line.strip() for line in bodies.get("leaf", "").splitlines()
                    if any(word in line for word in ("str\txzr", "orq\t", "callq\t"))],
                assembly_sha256=hashlib.sha256(assembly.read_bytes()).hexdigest()))
            if target == "darwin-arm64" and mode == "inline":
                run(name + "-measure-build", [LLVM["clang"], "-O2", SRC / "measure.c",
                    assembly, "-Wl,-undefined,dynamic_lookup", "-o", OUT / f"measure-O{level}"])
                measured = run(name + "-measure", [OUT / f"measure-O{level}"])
                deltas = {function: int(size) for function, size in re.findall(
                    r"^(\w+) sp_delta=(\d+)$", measured.stdout, re.M)}
                frames = matrix[-1]["frames"]
                fixed = {"leaf", "direct", "indirect", "runtime_call", "small_leaf", "body_check"}
                require(set(deltas) == fixed | {"dynamic_32768", "aligned", "loop_alloca_32"},
                        f"{name}: missing or unexpected SP measurements")
                require(all(deltas[function] == frames.get(function) for function in fixed),
                        f"{name}: fixed frame diagnostics disagree with measured SP deltas")
                for function, measurement in (("dynamic", "dynamic_32768"), ("loop_alloca", "loop_alloca_32")):
                    require(0 < frames.get(function, 0) <= 4096 < deltas[measurement]
                            and deltas[measurement] == 32768 + frames[function],
                            f"{name}: {function} must exceed the cap despite its small fixed frame")
                require(frames.get("aligned") == 65536
                        and 65536 <= deltas["aligned"] < 2 * 65536,
                        f"{name}: realigned SP delta is outside the fixed frame plus alignment padding")
                require(re.search(r"\band\s+(x\d+),\s*\1,\s*#0xffffffffffff0000", bodies["aligned"]),
                        f"{name}: missing realignment mask beyond the reported fixed frame")

for target, triple in TARGETS.items():
    run(target + "-final-frame", [LLVM["llc"], "-O2", f"-mtriple={triple}",
        "-stop-after=prologepilog", inputs["strict"], "-o", OUT / f"{target}-final.mir"])
    named = OUT / f"{target}-named.ll"
    named.write_text(base.replace('"probe-stack"="inline-asm"', '"probe-stack"="__mesh_probe"'))
    named_result = run(target + "-named-probe", [LLVM["llc"], "-O2", f"-mtriple={triple}",
        named, "-o", OUT / f"{target}-named.s"],
        (-6,) if "arm64" in target else (0,))
    if "arm64" in target:
        require("Unsupported stack probing method" in named_result.stderr,
                f"{target}: named probe must fail for the expected unsupported-method diagnostic")
no_call_opt = run("linux-x86_64-no-call-frame-opt", [LLVM["llc"], "-O2",
    "-mtriple=x86_64-unknown-linux-gnu", "-no-x86-call-frame-opt", inputs["strict"],
    "-o", OUT / "linux-x86_64-no-call-frame-opt.s"])

x86 = {(entry["optimization"], entry["mode"]): entry for entry in matrix
       if entry["target"] == "linux-x86_64"}
for level in ("O0", "O2"):
    require(x86[level, "inline"]["frames"].get("small_leaf", 0) == 0
            and x86[level, "strict"]["frames"].get("small_leaf", 0) > 0,
            f"linux-x86_64-{level}: noredzone must expose the otherwise omitted small leaf frame")
outgoing_mir = re.search(r"^name:\s+outgoing\n(.*?)(?=^\.\.\.)",
                         (OUT / "linux-x86_64-final.mir").read_text(), re.M | re.S)
require(outgoing_mir is not None, "linux-x86_64: missing outgoing final machine frame")
mir_sizes = {key: int(size) for key, size in re.findall(
    r"^\s+(stackSize|maxCallFrameSize):\s+(\d+)$", outgoing_mir[1], re.M)}
outgoing_body = functions((OUT / "linux-x86_64-O2-strict.s").read_text())["outgoing"]
before_call = re.split(r"\bcallq\s+many_args\b", outgoing_body)
require(len(before_call) == 2, "linux-x86_64: expected one outgoing call to many_args")
push_bytes = 8 * len(re.findall(r"\bpushq\b", before_call[0]))
require(0 < x86["O2", "strict"]["frames"].get("outgoing", 0) == mir_sizes.get("stackSize", 0)
        <= 4096 < mir_sizes.get("maxCallFrameSize", 0) < push_bytes
        and push_bytes == mir_sizes["stackSize"] + mir_sizes["maxCallFrameSize"],
        "linux-x86_64: outgoing argument pushes must exceed the cap omitted from the fixed frame")
require(x86["O0", "strict"]["frames"].get("outgoing") == push_bytes
        == frame_sizes(no_call_opt.stderr).get("outgoing"),
        "linux-x86_64: O0 and disabled call-frame optimization must account for the outgoing area")

flags = shlex.split(run("llvm-flags", [LLVM["llvm-config"], "--cxxflags", "--ldflags",
                                    "--libs", "--system-libs"]).stdout)
diagnostics = OUT / "diagnostics"
run("diagnostics-build", [LLVM["clang++"], SRC / "diagnostics.cpp", *flags, "-o", diagnostics])
(OUT / "cap.ll").write_text(base)
dynamic = re.search(r"define i64 @dynamic\(.*?\n\}", base, re.S).group()
(OUT / "dynamic-only.ll").write_text("declare ptr @llvm.stacksave.p0()\n" + dynamic +
    "\n" + base[base.index("attributes #0 ="):])
leaf = re.search(r"define i64 @leaf\(.*?\n\}", base, re.S).group()
for cap in (32783, 32784):
    path = OUT / f"leaf-cap-{cap}.ll"
    path.write_text("declare ptr @llvm.stacksave.p0()\n" + leaf + "\n" +
        base[base.index("attributes #0 ="):].replace('"4096"', f'"{cap}"', 1))
    for mode in ("object", "jit"):
        name = f"leaf-cap-{cap}-{mode}"
        result = run(name, [diagnostics, mode, path, "O2"], (2,) if cap == 32783 else (0,))
        check_diagnostics(name, result, mode, {"leaf"} if cap == 32783 else set(), cap)
for level in (0, 2):
    for mode in ("object", "jit", "object-error", "jit-error"):
        name = f"diagnostics-{mode}-O{level}"
        result = run(name, [diagnostics, mode, OUT / "cap.ll", f"O{level}"], (2,))
        check_diagnostics(name, result, mode,
                          {"leaf", "direct", "indirect", "runtime_call", "aligned", "body_check", "outgoing"}, 4096)
    for mode in ("object", "jit"):
        name = f"dynamic-only-{mode}-O{level}"
        result = run(name, [diagnostics, mode, OUT / "dynamic-only.ll", f"O{level}"])
        check_diagnostics(name, result, mode, set(), 4096)
    obj = OUT / f"unwind-O{level}.o"
    run(f"unwind-codegen-O{level}", [LLVM["llc"], f"-O{level}", "-filetype=obj",
        SRC / "unwind.ll", "-o", obj])
    run(f"unwind-asm-O{level}", [LLVM["llc"], f"-O{level}", "-filetype=asm",
        SRC / "unwind.ll", "-o", OUT / f"unwind-O{level}.s"])
    exe = OUT / f"unwind-O{level}"
    run(f"unwind-build-O{level}", [RUSTC, "--edition=2021", "-O", "-C", "panic=unwind",
        "-C", f"link-arg={obj}", SRC / "unwind.rs", "-o", exe])
    result = run(f"unwind-runtime-O{level}", [exe])
    unwinds = [tuple(map(int, row)) for row in re.findall(
        r"^indirect=(\d+) caught=(\d+) entered=(\d+) dropped=(\d+) failure_slack=(\d+)$",
        result.stdout, re.M)]
    require(len(unwinds) == 2 and {row[0] for row in unwinds} == {0, 1},
            f"unwind-runtime-O{level}: both direct and indirect runs must report cleanup")
    require(all(caught == 1 and 0 < entered == dropped < 256 * 1024 // 4096
                and 0 < slack < 64 * 1024 for _, caught, entered, dropped, slack in unwinds)
            and result.stdout.splitlines()[-1] == "process_after_unwinds=alive",
            f"unwind-runtime-O{level}: unmatched cleanup, uncaught unwind, or out-of-bounds failure slack")

(OUT / "results.json.tmp").write_text(json.dumps(dict(metadata=metadata, matrix=matrix,
    observations=[r for r in records if "diagnostics-" in r["name"] or "-measure" in r["name"]
                  or "dynamic-only-" in r["name"] or "unwind-runtime" in r["name"]
                  or "-named-probe" in r["name"] or "no-call-frame" in r["name"]
                  or "leaf-cap-" in r["name"]]), indent=2) + "\n")
(OUT / "results.json.tmp").replace(OUT / "results.json")
print(f"PASS: {len(matrix)} codegen cases; diagnostic/publication outcomes, stack gaps, and unwind cleanup checked.")
print("Runtime probes ran only on Darwin arm64. Actor containment is not qualified.")
print(f"Artifacts: {OUT}")
