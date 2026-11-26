"""Boot the pinned GNU AArch64 guest and verify named effects in the exact Jobs ELF."""

import argparse
from hashlib import sha256
import json
from pathlib import Path
import stat
import sys
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
DRIVER = ROOT / "proof/jobs-parity/linux-regression"
sys.path.insert(0, str(DRIVER))
import pack as image
import run as guest

BASE = ROOT / "target/jobs-perf-work/canonical-recovery/guest-inputs"
ASSETS = ROOT / "target/linux-queue-perf"
ELF_SHA = json.loads((HERE / "link-inputs.json").read_text())["layout_elf_sha256"]


def package(output, profile, arms, mesh, mesh_sha, assets, *, base_path, init_path,
            diagnostic_unset_inetrc):
    base = base_path.read_bytes()
    assert image.digest(base) == image.BASE_SHA256
    assert image.digest((assets / "kernel.Image").read_bytes()) == image.KERNEL_SHA256
    binary = mesh.read_bytes()
    assert image.digest(binary) == mesh_sha == ELF_SHA
    init = init_path.read_bytes()
    assert image.digest(init) == image.INIT_SHA256
    driver = (DRIVER / "trial.py").read_text()
    start = driver.index("def run(config, root, evidence):")
    end = driver.index("\ndef main():", start)
    driver = driver[:start] + "from package_guest import run\n" + driver[end:]
    # Load the probe after trial's classes are initialized and use the same module
    # instance for both the guest entry point and the probe.
    driver = driver.replace("from package_guest import run\n",
                            'sys.modules["trial"] = sys.modules[__name__]\nfrom package_guest import run\n')
    compile(driver, "trial.py", "exec")
    overlay = bytearray()
    files = {
        "init": (init, 0o755),
        "proof/run.sh": ((DRIVER / "run.sh").read_bytes(), 0o755),
        "proof/mesh-current": (binary, 0o755),
        "proof/trial.py": (driver.encode(), 0o644),
        "proof/package_guest.py": ((HERE / "package_guest.py").read_bytes(), 0o644),
        "proof/jobs_parity_fault.py": ((ROOT / "scripts/lib/jobs_parity_fault.py").read_bytes(), 0o644),
        "proof/jobs-parity.py": ((ROOT / "scripts/jobs-parity.py").read_bytes(), 0o644),
        "proof/jobs-parity-profiles.json": ((ROOT / "scripts/jobs-parity-profiles.json").read_bytes(), 0o644),
    }
    for name in ("fence.sql", "application.sql", "oban_runtime.py", "proof_transport.py"):
        files[f"proof/{name}"] = ((DRIVER / name).read_bytes(), 0o644)
    image.add(overlay, "proof", stat.S_IFDIR | 0o755)
    config = {"elf_sha256": ELF_SHA, "kernel_sha256": image.KERNEL_SHA256, "hashes": {}}
    for name, (data, mode) in files.items():
        image.add(overlay, name, stat.S_IFREG | mode, data)
        config["hashes"][name] = sha256(data).hexdigest()
    image.add(overlay, "proof/config.json", stat.S_IFREG | 0o644, json.dumps(config).encode())
    image.add(overlay, "TRAILER!!!", 0)
    with output.open("xb") as stream:
        stream.write(base)
        stream.write(bytes(-len(base) % 4))
        stream.write(overlay)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("elf", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    guest.pack = package
    return guest.run(SimpleNamespace(
        output=args.output.resolve(), profile="slow", arms=["mesh"], mesh=args.elf.resolve(),
        mesh_sha256=ELF_SHA, assets=ASSETS, guest_base=BASE / "base.cpio",
        guest_init=BASE / "stock-jobs-smoke/init", diagnostic_unset_inetrc=False,
    ))


if __name__ == "__main__":
    raise SystemExit(main())
