"""Generate or replay the local correctness receipt for the transferred pool."""
import argparse
import gzip
from hashlib import file_digest, sha256
import json
from pathlib import Path
import re
import subprocess

SOURCE = "1e70f18d624a841740d3bbb3d0fd09b269257f68"
PRODUCT = "916cfe532d76616ea28856ff9ca8a5132fecc556"
PRODUCT_PATHS = (
    "Cargo.toml", "Cargo.lock", ".cargo", "compiler", "scripts", "docs",
    "examples", "packages", "proof", "tests", "website/docs/docs/databases",
)
FILES = (
    "compiler/mesh-rt/src/db/pg.rs",
    "compiler/mesh-rt/src/db/pool.rs",
    "website/docs/docs/databases/index.md",
)
SERVERS = {
    "pg17": "17.11",
    "postgres-fence": "17.11",
    "pg-18.7-retry": "18.7",
    "fence-18.7": "18.7",
    "pg-19.0-retry": "19.0",
    "fence-19.0": "19.0",
}
REQUIRED = (
    "format", "pool", "pg-wire", "pg17", "postgres-fence", "workspace-build",
    "workspace-correctness", "pg-18.7-retry", "fence-18.7", "pg-19.0-retry", "fence-19.0",
)
POSTGRES_ARCHIVES = {
    "18.7": "441dfae4280cae3e0425e5f876f80559fb0731c140f1aae22b3e5a54d0411bfd",
    "19.0": "a89fd70efbad6a08c7dbed8d4872e3af30170d9e5ca9dc7906523be95b4e692d",
}
SCRIPTS = ("gates.sh", "compat.sh", "compat-rebuilt.sh", "build-postgres.sh")
REMAINDER = Path(__file__).with_name("workspace_remainder.py")


def digest(data):
    return sha256(data).hexdigest()


def sanitize(data):
    data = re.sub(
        rb"(?i)([a-z][a-z0-9+.-]*://)[^\s/<>\x22\x27]+@",
        rb"\1[redacted]@", data,
    )
    data = re.sub(
        rb"(?i)([?&](?:password|token|api[_-]?key|signature|credential|sig|x-amz-[\w-]+)=)[^&\s<>\x22\x27]+",
        rb"\1[redacted]", data,
    )
    data = re.sub(
        rb"(?im)^(\s*(?:proxy-)?authorization:\s*)[^\r\n]+",
        rb"\1[redacted]", data,
    )
    data = re.sub(
        rb"""(?i)(\bpassword\s*(?:=\s*)?)['"][^'"]+['"]""",
        rb"\1[redacted]", data,
    )
    return re.sub(
        rb"(?ms)-----BEGIN [^\r\n]*PRIVATE KEY-----.*?-----END [^\r\n]*PRIVATE KEY-----",
        b"[redacted private key]", data,
    )


def verify_source(checkout):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(checkout), *args])

    if git("rev-parse", "HEAD").decode().strip() != SOURCE:
        raise ValueError("verification used an unexpected checkout")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("verification checkout contains source changes")
    git("diff", "--exit-code", SOURCE, PRODUCT, "--", *PRODUCT_PATHS)


def read_logs(directory, archived):
    suffix = ".log.gz" if archived else ".log"
    logs = {}
    for path in sorted(directory.glob("*" + suffix)):
        raw = path.read_bytes()
        data = gzip.decompress(raw) if archived else raw
        clean = sanitize(data)
        if archived and clean != data:
            raise ValueError(f"unsanitized log in archive at {path.name}")
        logs[path.name.removesuffix(suffix)] = clean
    status = (directory / "status").read_bytes()
    exits = {}
    for line in status.decode().splitlines():
        name, code = line.split()
        if name in exits or name not in logs:
            raise ValueError(f"duplicate or missing gate log at {name}")
        exits[name] = int(code)
    return logs, status, exits


def summarize(name, data, code):
    body = data.decode(errors="replace")
    item = {"name": name, "exit": code, "log_sha256": digest(data)}
    suites = re.findall(
        r"^test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;",
        body, re.MULTILINE,
    )
    if suites:
        item.update({
            key: sum(map(int, counts))
            for key, counts in zip(("passed", "failed", "ignored"), zip(*suites))
        })
    versions = re.findall(r"^Disposable Jobs proof server: PostgreSQL (\d+\.\d+)", body, re.MULTILINE)
    if versions:
        item["server_version"] = versions[0]
    roles = re.findall(r"^Scoped Jobs role proof passed on (\w+)", body, re.MULTILINE)
    if roles:
        item["role_databases"] = roles
    if code == 0 and name in SERVERS:
        if versions != [SERVERS[name]] or f"Jobs proof checkout: {SOURCE}" not in body:
            raise ValueError(f"wrong server or source at {name}")
        if name.startswith("fence-") or name == "postgres-fence":
            if "Jobs effect fence proof passed." not in body:
                raise ValueError(f"missing effect fence result at {name}")
        if name in ("pg-18.7-retry", "pg-19.0-retry"):
            if set(roles) != {"jobs_role_fresh", "jobs_role_upgrade"}:
                raise ValueError(f"missing fresh or upgrade role proof at {name}")
    if code == 0 and item.get("failed"):
        raise ValueError(f"failed tests in successful gate {name}")
    return item


def receipt(lane, archived=None, archive=None):
    checkout = lane / "source"
    verify_source(checkout)
    directory = archived or lane / "logs"
    scripts = archived or lane
    logs, status, exits = read_logs(directory, archived is not None)
    toolchain = sanitize((directory / "toolchain.txt").read_bytes())
    gates = [summarize(name, data, exits.get(name)) for name, data in logs.items()]
    builds = {}
    for version, checksum in POSTGRES_ARCHIVES.items():
        filename = f"postgresql-{version}.tar.bz2"
        if archived is None:
            with (lane / "postgres" / filename).open("rb") as stream:
                if file_digest(stream, "sha256").hexdigest() != checksum:
                    raise ValueError(f"unexpected source archive {filename}")
        output = logs.get(f"build-pg-{version}", b"").decode(errors="replace")
        builds[version] = {
            "archive": filename,
            "archive_sha256": checksum,
            "install_version_confirmed": (
                output.startswith(f"{filename}: OK\n")
                and output.rstrip().endswith(f"postgres (PostgreSQL) {version}")
            ),
        }
    if any(exits.get(name) == 0 and not builds[version]["install_version_confirmed"]
           for name, version in SERVERS.items() if version in builds):
        raise ValueError("compatibility evidence is missing its matching source build")
    if archive:
        archive.mkdir(parents=True, exist_ok=False)
        for name, data in logs.items():
            (archive / f"{name}.log.gz").write_bytes(gzip.compress(data, mtime=0))
        (archive / "status").write_bytes(status)
        (archive / "toolchain.txt").write_bytes(toolchain)
        for script in SCRIPTS:
            (archive / script).write_bytes((scripts / script).read_bytes())
    return {
        "source": SOURCE,
        "integrated_product": PRODUCT,
        "matching_product_paths": PRODUCT_PATHS,
        "files": {path: digest((checkout / path).read_bytes()) for path in FILES},
        "scope": "macOS ARM64 local correctness",
        "excluded": [
            "standalone compiler performance gate",
            "e2e_performance compile timing test",
            "Linux native qualification",
        ],
        "gates": gates,
        "required_gates": {name: exits.get(name) for name in REQUIRED},
        "workspace_remainder_exit": exits.get("workspace-remainder"),
        "postgres_source_builds": builds,
        "scripts_sha256": {name: digest((scripts / name).read_bytes()) for name in SCRIPTS},
        "workspace_remainder_script_sha256": digest(REMAINDER.read_bytes()),
        "toolchain_sha256": digest(toolchain),
        "status_sha256": digest(status),
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lane", type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--archive", type=Path, help="write sanitized logs and replay scripts to a new directory")
    mode.add_argument("--archived", type=Path, help="regenerate the receipt using retained logs")
    args = parser.parse_args()
    print(json.dumps(receipt(args.lane.resolve(), args.archived, args.archive), indent=2, sort_keys=True))
