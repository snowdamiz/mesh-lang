"""Add a gated native actor termination callback to a copy of the benchmark fixture."""
from pathlib import Path
import shutil
import sys

source, out = (Path(p).resolve() for p in sys.argv[1:])
out.mkdir()
shutil.copytree(source / "fixture", out / "fixture")
shutil.copytree(source / "replay", out / "replay")
fixture = out / "fixture"
main = fixture / "main.mpl"
s = main.read_text()
s = s.replace("import realtime_us, worker_pid", "import realtime_us, worker_pid, arm_teardown")
s = s.replace("fn handle(context :: String) -> String do\n", """fn handle(context :: String) -> String do
  if arm_teardown() != 0 do panic("failed to register actor teardown probe") end
""")
s = s.replace("  let runner = Jobs.start(pool, ordinary_config, handle)?", """  let runner_config = if Env.get("COMBINED_TEARDOWN_PUBLIC", "") == "1" do trusted else ordinary_config end
  let runner = Jobs.start(pool, runner_config, handle)?""")
main.write_text(s)
decl = fixture / "worker_identity.mpl"
with decl.open("a") as target:
    target.write('\n@native("mesh_parity_arm_teardown")\npub fn arm_teardown() -> Int\n')
native = fixture / "worker_identity.c"
with native.open("a") as target:
    target.write("\n" + Path(__file__).with_name("teardown.c").read_text())
