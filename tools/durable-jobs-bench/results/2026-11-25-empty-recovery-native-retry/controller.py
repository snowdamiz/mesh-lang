"""Bind the original pilot driver to a separately sealed retry."""
from pathlib import Path
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
ORIGINAL = HERE.with_name("2026-11-25-empty-recovery-native")
sys.path.insert(0, str(ORIGINAL))
import controller as pilot

STAGED = [HERE / name for name in ("launch.sh", "prepare-launch.py", "contract.py", "observe.py")]
STAGED += [ORIGINAL / "probe.py", pilot.CANDIDATE_FILE]
ORIGINAL_PACKET = "b53ee2facf8e01ae8cff0904c01dfd207a1d1b196e2e8fe413e63dd5ec082122"


def check_retry(cfg):
    subprocess.run(["bash", "-n", HERE / "launch.sh"], check=True)
    # Use the standalone prepare script as staged. Its directory has at most
    # three parents in production, and the checkout is absent from sys.path.
    with tarfile.open(pilot.PUBLIC / "source.tar.gz") as archive:
        name, = [p for p in archive.getnames() if p.endswith("/replay/native-run")]
        product_launcher = archive.extractfile(name).read().decode()
    for lane in pilot.PHASES:
        argv = cfg["lanes"][lane]["inventory"]["engines"]["mesh"]["argv"]
        assert cfg["lanes"][lane]["inventory"]["engines"]["mesh"]["verified_files"][argv[-4]] == (
            pilot.run.digest(HERE / "launch.sh"))
        subprocess.run(["bash", "-n"], input=pilot.rewrite(product_launcher, Path(argv[-3]).parents[5], lane),
                       text=True, check=True)
    for path in HERE.glob("*.py"):
        compile(path.read_bytes(), str(path), "exec")
    receipt = pilot.run.read_json(HERE / "local-shell-proof.json")
    assert receipt["uid"] != 0 and receipt["server_version"] == "170011"
    assert not receipt["timed"] and receipt["cases"] == 24 and receipt["failure_uses_named_script"]
    print(f"{pilot.ID}: staged shell proof verified; packet {pilot.run.digest(pilot.PACKET)}")


def main():
    assert pilot.run.digest(pilot.PACKET) == ORIGINAL_PACKET
    pilot.check_packet()
    if sys.argv[1:] == ["check"]:
        pilot.proof(pilot.config())
    pilot.HERE = HERE
    pilot.ID = HERE.name
    pilot.PACKET = HERE / "controller-packet.json"
    pilot.OUT = HERE / "evidence/native"
    pilot.STAGED = STAGED
    pilot.proof = check_retry
    return pilot.main()


if __name__ == "__main__":
    sys.exit(main())
