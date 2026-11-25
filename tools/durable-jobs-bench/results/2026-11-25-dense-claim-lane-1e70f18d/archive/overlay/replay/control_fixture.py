"""Add a test-only file rendezvous for public local queue control."""
from pathlib import Path
import shutil
import sys

overlay, dest = map(Path, sys.argv[1:3])
dest.mkdir()
shutil.copytree(overlay / "fixture", dest / "fixture")
(dest / "replay").mkdir()
shutil.copyfile(overlay / "replay/link.sh", dest / "replay/link.sh")
main = dest / "fixture/main.mpl"
text = main.read_text()
line = '    if status_path != "" && File.exists(status_path <> ".request") do\n'
assert text.count(line) == 1
main.write_text(text.replace(line, '''    if status_path != "" && File.exists(status_path <> ".control-request") do
      let command = File.read(status_path <> ".control-request")?
      let report = Jobs.local_control(runner, command)?
      let _ = File.rename(status_path <> ".control-request", status_path <> ".control-pending")?
      let _ = File.write(status_path <> ".control-pending", report)?
      let _ = File.rename(status_path <> ".control-pending", status_path <> ".control-result")?
      ()
    end
''' + line))
