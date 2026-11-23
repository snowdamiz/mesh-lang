"""Relink the captured, qualified fixture object with the candidate runtime."""
from pathlib import Path
from hashlib import sha256
import os
import shlex
import subprocess

here = Path(__file__).resolve().parent
captured = here.parent / "dense-cpu-sampling"
libgcc = Path(os.environ["JOBS_LINUX_PROOF_LIBGCC"])
runtime = here / "linux-target/aarch64-unknown-linux-gnu/release/libmesh_rt.a"
output = here / "mesh"
line, = [line for line in (captured / "link.log").read_text().splitlines()
         if line.startswith("ld.lld ") and " -dynamic-linker " in line]
args = shlex.split(line)[1:]
args[args.index("-o") + 1] = str(output)
args = [str(captured / "fixture.o") if arg.endswith("/relink-stripped.o")
        else str(runtime) if arg.endswith("/libmesh_rt.a")
        else str(libgcc) if arg.endswith("/libgcc_s.so.1")
        else str(captured / arg) if arg.startswith("zig-global/")
        else arg for arg in args]
assert sha256((captured / "relink-stripped").read_bytes()).hexdigest() == (
    "fe719b77c033f0c08e61138291eb119487c796476e23fe9fb544711dd50233c8")
assert sha256(libgcc.read_bytes()).hexdigest() == (
    "a886d874bfcc27b5b3de47504eb861af8f15cf8c2aea160bd3b973e1a1a95255")
command = [os.environ["JOBS_LINUX_CROSS_ZIG"], "ld.lld", *args]
print(shlex.join(command), flush=True)
subprocess.run(command, cwd=here, check=True)
for path in (captured / "fixture.o", runtime, output):
    print(sha256(path.read_bytes()).hexdigest(), path)
