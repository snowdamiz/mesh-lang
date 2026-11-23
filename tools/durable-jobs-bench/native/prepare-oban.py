"""Build the frozen Oban Basic module with the regression session adapter."""

from pathlib import Path
import sys
import tarfile

archive, proof, output = map(Path, sys.argv[1:])
with tarfile.open(archive) as frozen:
    with frozen.extractfile("oban-build/src/jobs_parity_oban.ex") as source:
        module = source.read().decode()
anchor = '  def run([action]) when action in ["migrate", "enqueue", "run"] do'
if module.count(anchor) != 1:
    raise ValueError("frozen Oban source changed")
broker = (proof / "oban-session.ex.in").read_text()
(output / "oban-source/lib/jobs_parity_oban.ex").write_text(module.replace(anchor, broker + "\n" + anchor))
entry = (proof / "fixture.exs").read_text()
load_session = 'if args == ["session"] do\n  Code.require_file("/proof/oban-session.ex")\nend\n\n'
if entry.count(load_session) != 1:
    raise ValueError("frozen Oban launcher changed")
entry = entry.replace(load_session, "")
# Non-Linux dev smokes have no procfs. Linux preflight always demands its report.
entry = entry.replace('if args in [["preflight"], ["run"], ["session"]] do',
                      'if File.exists?("/proc/self/cmdline") and args in [["preflight"], ["run"], ["session"]] do')
(output / "oban-entry.exs").write_text(entry)
