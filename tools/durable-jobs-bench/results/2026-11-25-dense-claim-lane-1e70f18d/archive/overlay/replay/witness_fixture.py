"""Build the ordinary fixture with a termination hook for the correctness witness."""
from pathlib import Path
import shutil
import sys

packet, out = map(lambda name: Path(name).resolve(), sys.argv[1:3])
out.mkdir()
shutil.copytree(packet / "fixture", out / "fixture")
(out / "replay").mkdir()
shutil.copyfile(packet / "replay/link.sh", out / "replay/link.sh")
main = out / "fixture/main.mpl"
src = main.read_text()
for old, new in (
    ("import realtime_us, worker_pid", "import realtime_us, worker_pid, arm_teardown"),
    ("fn handle(context :: String) -> String do\n",
     'fn handle(context :: String) -> String do\n  if arm_teardown() != 0 do panic("cannot register teardown witness") end\n'),
):
    assert src.count(old) == 1
    src = src.replace(old, new)
main.write_text(src)
decl = out / "fixture/worker_identity.mpl"
with decl.open("a") as f:
    f.write('\n@native("mesh_parity_arm_teardown")\npub fn arm_teardown() -> Int\n')
c = out / "fixture/worker_identity.c"
with c.open("a") as f:
    f.write("\n" + (packet / "replay/teardown.c").read_text())
