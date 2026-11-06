#!/usr/bin/env python3
"""Bounded LLVM 21 experiments. Outputs stay in the assigned checkout."""
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import resource
import shlex
import subprocess

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "proof/stack-containment"
OUT = ROOT / ".tmp/stack-containment/run"
OUT.mkdir(parents=True, exist_ok=True)
(OUT / "results.json").unlink(missing_ok=True)
LLVM = Path(os.environ.get("LLVM_SYS_211_PREFIX", "/opt/homebrew/opt/llvm@21")) / "bin"
RUSTC = Path("/Users/andreyyurlov/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc")
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
    "base_commit": run("head", ["git", "rev-parse", "HEAD"]).stdout.strip(),
    "host": platform.platform(),
    "host_page_size": os.sysconf("SC_PAGE_SIZE"),
    "llvm": run("llvm-version", [LLVM / "llc", "--version"]).stdout,
    "rustc": run("rust-version", [RUSTC, "--version"]).stdout.strip(),
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
                run(name + "-opt", [LLVM / "opt", "-passes=default<O2>",
                    f"-mtriple={triple}", "-S", source, "-o", ir])
            assembly = OUT / f"{name}.s"
            result = run(name, [LLVM / "llc", f"-O{level}", f"-mtriple={triple}",
                         "-filetype=asm", ir, "-o", assembly])
            bodies = functions(assembly.read_text())
            matrix.append(dict(target=target, optimization=f"O{level}", mode=mode,
                frames=frame_sizes(result.stderr),
                outgoing_pushes=len(re.findall(r"\bpushq\b", bodies.get("outgoing", ""))),
                leaf_probe_instructions=[line.strip() for line in bodies.get("leaf", "").splitlines()
                    if any(word in line for word in ("str\txzr", "orq\t", "callq\t"))],
                assembly_sha256=hashlib.sha256(assembly.read_bytes()).hexdigest()))
            if target == "darwin-arm64" and mode == "inline":
                run(name + "-measure-build", [LLVM / "clang", "-O2", SRC / "measure.c",
                    assembly, "-Wl,-undefined,dynamic_lookup", "-o", OUT / f"measure-O{level}"])
                run(name + "-measure", [OUT / f"measure-O{level}"])

for target, triple in TARGETS.items():
    run(target + "-final-frame", [LLVM / "llc", "-O2", f"-mtriple={triple}",
        "-stop-after=prologepilog", inputs["strict"], "-o", OUT / f"{target}-final.mir"])
    named = OUT / f"{target}-named.ll"
    named.write_text(base.replace('"probe-stack"="inline-asm"', '"probe-stack"="__mesh_probe"'))
    named_result = run(target + "-named-probe", [LLVM / "llc", "-O2", f"-mtriple={triple}",
        named, "-o", OUT / f"{target}-named.s"],
        (-6,) if "arm64" in target else (0,))
    if "arm64" in target:
        assert "Unsupported stack probing method" in named_result.stderr
run("linux-x86_64-no-call-frame-opt", [LLVM / "llc", "-O2",
    "-mtriple=x86_64-unknown-linux-gnu", "-no-x86-call-frame-opt", inputs["strict"],
    "-o", OUT / "linux-x86_64-no-call-frame-opt.s"])

flags = shlex.split(run("llvm-flags", [LLVM / "llvm-config", "--cxxflags", "--ldflags",
                                    "--libs", "--system-libs"]).stdout)
diagnostics = OUT / "diagnostics"
run("diagnostics-build", [LLVM / "clang++", SRC / "diagnostics.cpp", *flags, "-o", diagnostics])
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
        run(f"leaf-cap-{cap}-{mode}", [diagnostics, mode, path, "O2"],
            (2,) if cap == 32783 else (0,))
for level in (0, 2):
    for mode in ("object", "jit", "object-error", "jit-error"):
        run(f"diagnostics-{mode}-O{level}", [diagnostics, mode, OUT / "cap.ll", f"O{level}"], (2,))
    for mode in ("object", "jit"):
        run(f"dynamic-only-{mode}-O{level}", [diagnostics, mode, OUT / "dynamic-only.ll", f"O{level}"])
    obj = OUT / f"unwind-O{level}.o"
    run(f"unwind-codegen-O{level}", [LLVM / "llc", f"-O{level}", "-filetype=obj",
        SRC / "unwind.ll", "-o", obj])
    run(f"unwind-asm-O{level}", [LLVM / "llc", f"-O{level}", "-filetype=asm",
        SRC / "unwind.ll", "-o", OUT / f"unwind-O{level}.s"])
    exe = OUT / f"unwind-O{level}"
    run(f"unwind-build-O{level}", [RUSTC, "--edition=2021", "-O", "-C", "panic=unwind",
        "-C", f"link-arg={obj}", SRC / "unwind.rs", "-o", exe])
    run(f"unwind-runtime-O{level}", [exe])

(OUT / "results.json").write_text(json.dumps(dict(metadata=metadata, matrix=matrix,
    observations=[r for r in records if "diagnostics-" in r["name"] or "-measure" in r["name"]
                  or "dynamic-only-" in r["name"] or "unwind-runtime" in r["name"]
                  or "-named-probe" in r["name"] or "no-call-frame" in r["name"]
                  or "leaf-cap-" in r["name"]]), indent=2) + "\n")
print(f"PASS: {len(matrix)} codegen cases and expected diagnostic outcomes reproduced.")
print("Darwin measurement and synthetic unwind probes completed. Actor containment is not qualified.")
print(f"Artifacts: {OUT}")
