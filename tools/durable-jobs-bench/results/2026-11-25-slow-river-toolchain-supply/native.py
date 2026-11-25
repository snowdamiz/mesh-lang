"""Validate and provision the supplied SDK and module cache inside native SETUP."""

import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile


ACTIVE_ARM = Path("/var/tmp/mesh-durable-jobs-matrix/active-arm.lock")
NATIVE = Path("/var/lib/durable-jobs-bench/native")
SDK_SHA = "1153d3d50e0ac764b447adfe05c2bcf08e889d42a02e0fe0259bd47f6733ad7f"
SDK_SIZE = 66861266
STOCK_SHA = "a23130f74210946291ed3609797e5b218ece5a7bd70df318851cf91e634da1e1"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def relative(name):
    path = PurePosixPath(name)
    require(str(path) == name and not path.is_absolute() and ".." not in path.parts,
            f"unsafe archive path: {name}")
    return path


def elf(path):
    with path.open("rb") as stream:
        header = stream.read(20)
    require(header[:6] == b"\x7fELF\x02\x01" and header[16:18] in (b"\x02\x00", b"\x03\x00")
            and header[18:20] == b"\x3e\x00" and os.access(path, os.X_OK), f"wrong Linux AMD64 executable: {path}")


def unpack_payload(root, pins):
    seen = set()
    with tarfile.open(fileobj=sys.stdin.buffer, mode="r|") as archive:
        for member in archive:
            name = member.name
            require(member.isfile() and name in pins and name not in seen
                    and member.size == pins[name]["size"], f"unexpected payload member: {name}")
            out = root / relative(name)
            out.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, out.open("xb") as target:
                shutil.copyfileobj(source, target)
            require(sha(out) == pins[name]["sha256"], f"payload digest differs: {name}")
            seen.add(name)
    require(seen == pins.keys(), "incomplete delivery")


def unpack_sdk(archive_path, destination):
    require(archive_path.stat().st_size == SDK_SIZE and sha(archive_path) == SDK_SHA, "official SDK digest differs")
    seen = set()
    with tarfile.open(archive_path, mode="r:gz") as archive:
        for member in archive:
            rel = relative(member.name)
            require(rel.parts and rel.parts[0] == "go" and rel not in seen, f"unexpected SDK member: {rel}")
            out = destination / rel
            seen.add(rel)
            if member.isdir():
                out.mkdir(parents=True, exist_ok=True)
            else:
                require(member.isfile(), f"unexpected SDK member type: {rel}")
                out.parent.mkdir(parents=True, exist_ok=True)
                with archive.extractfile(member) as source, out.open("xb") as target:
                    shutil.copyfileobj(source, target)
                out.chmod(0o755 if member.mode & 0o111 else 0o644)
    go = destination / "go"
    require((go / "VERSION").read_text().splitlines()[0] == "go1.26.4", "wrong SDK VERSION file")
    for name in ("bin/go", "pkg/tool/linux_amd64/compile", "pkg/tool/linux_amd64/link"):
        elf(go / name)
    return go / "bin/go"


def check_host(data):
    require(os.geteuid() == 0 and sys.platform == "linux" and os.uname().machine == "x86_64",
            "native supply requires the controller's Linux AMD64 account")
    boot, kernel, disk = data["host"]
    mask = os.sched_getaffinity(0)
    require(len(mask) == 4 and {0, 1} <= mask and os.uname().release == kernel
            and Path("/proc/sys/kernel/random/boot_id").read_text().strip() == boot, "native host, boot or CPU roster differs")
    mount = subprocess.check_output(["/usr/bin/findmnt", "-rn", "-o", "MAJ:MIN",
                                     "--target", data["storage"]], text=True).strip()
    require(mount == disk, "retained PostgreSQL mount differs")
    for path in Path("/proc").glob("[0-9]*/comm"):
        try:
            name = path.read_text().strip()
        except (FileNotFoundError, ProcessLookupError):
            continue
        require(name not in {"postgres", "go", "cargo", "rustc", "cc1", "cc1plus", "clang", "gcc", "make"},
                f"host is busy with {name}")
    require(sha(Path(data["retained_stock_elf"])) == STOCK_SHA, "retained cap-71 stock ELF differs")


def install(data, work, destination):
    payload = work / "payload"
    pin_files = data["payload"]
    unpack_payload(payload, pin_files)
    deployed = work / "publish"
    deployed.mkdir(mode=0o700)
    compiler = unpack_sdk(payload / "go1.26.4.linux-amd64.tar.gz", deployed)
    modcache = deployed / "mod"
    (payload / "mod").rename(modcache)
    build = work / "stock-elf"
    (work / "tmp").mkdir()
    env = dict(PATH="/usr/bin:/bin", LC_ALL="C", GOOS="linux", GOARCH="amd64", GOAMD64="v1",
               CGO_ENABLED="0", GOPROXY="off", GOSUMDB="off", GOVCS="*:off", GOTOOLCHAIN="local",
               GOWORK="off", GOENV="off", GOFLAGS="", GOTELEMETRY="off", GOPATH=str(work / "gopath"),
               GOMODCACHE=str(modcache), GOCACHE=str(work / "build-cache"), GOTMPDIR=str(work / "tmp"))
    stock = payload / "stock"
    version = subprocess.check_output([compiler, "version"], cwd=stock, env=env, text=True).strip()
    require(version == "go version go1.26.4 linux/amd64", "wrong executable Go version")
    verify = [compiler, "mod", "verify"]
    subprocess.run(verify, cwd=stock, env=env, check=True, stdout=sys.stderr)
    subprocess.run([compiler, "build", "-mod=readonly", "-trimpath", "-buildvcs=false", "-o", build, "."],
                   cwd=stock, env=env, check=True, stdout=sys.stderr)
    subprocess.run(verify, cwd=stock, env=env, check=True, stdout=sys.stderr)
    elf(build)
    require(sha(build) == STOCK_SHA, "native stock rebuild differs from retained SLOW ELF")
    info = subprocess.check_output([compiler, "version", "-m", build], cwd=stock, env=env, text=True).splitlines()[1:]
    require(info == data["build_info"], "native stock module or build metadata differs")
    for name, pin in pin_files.items():
        artifact = deployed / name if name.startswith("mod/") else payload / name
        require(sha(artifact) == pin["sha256"], f"sealed build input changed: {name}")
    binding = dict(request_sha256=data["proof_request_sha256"], compiler=str(destination / "go/bin/go"),
                   compiler_sha256=sha(compiler), module_cache=str(destination / "mod"))
    receipt = dict(supply_request_sha256=data["supply_request_sha256"], module_inventory_sha256=data["inventory_sha256"],
                   sdk_sha256=SDK_SHA, stock_elf_sha256=STOCK_SHA, host=data["host"], toolchain_binding=binding,
                   compiler_version=version, stock_build_info=info)
    with (deployed / "supply-receipt.json").open("x") as stream:
        json.dump(receipt, stream, indent=2, sort_keys=True)
        stream.write("\n")
    require(not os.path.lexists(destination), "native destination already exists")
    deployed.rename(destination)
    return receipt


def main():
    os.umask(0o077)
    data = json.loads(sys.argv[1])
    with ACTIVE_ARM.open("rb") as active, (NATIVE / "native.lock").open("rb") as native:
        for lock in (active, native):
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        check_host(data)
        require(data["sdk_sha256"] == SDK_SHA and data["stock_sha256"] == STOCK_SHA, "supply identity differs")
        seal = data["supply_request_sha256"]
        require(len(seal) == 64 and all(c in "0123456789abcdef" for c in seal), "invalid supply request seal")
        root = NATIVE / "toolchain"
        destination = root / f"slow-river-readiness-go1.26.4-{seal[:16]}"
        require(root.resolve(strict=True) == root and root.stat().st_uid == 0
                and not root.stat().st_mode & 0o022 and not os.path.lexists(destination),
                "unexpected native toolchain directory or a retained installation already exists")
        with tempfile.TemporaryDirectory(prefix=".slow-river-supply-", dir=root) as tmp:
            receipt = install(data, Path(tmp), destination)
        print(json.dumps(receipt, indent=2, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
