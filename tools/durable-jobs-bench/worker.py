"""Run one arm on the native host and export a redacted copy of its evidence."""

import argparse
import fcntl
from fnmatch import fnmatchcase
import gzip
import io
import json
import os
from pathlib import Path
import platform
import pwd
import re
import signal
import subprocess
import sys
import tarfile
import time


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
BUILDERS = {"cargo", "rustc", "cc1", "cc1plus", "zig", "clang", "gcc", "go", "make", "ninja", "cmake",
            "apt", "apt-get", "dpkg"}
SAFE_EXTENSIONS = {".json", ".jsonl", ".csv", ".log", ".txt", ".tsv"}
PRIVATE_NAME = re.compile(
    r"(?:^|[._-])(?:password|secret|credentials?|pgpass|private|id_ed25519|token|authorization|api[_-]?key)"
    r"(?:$|[._-])", re.I)
PRIVATE_VALUE = re.compile(
    r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----|\b(?:postgres(?:ql)?|ecto)://|"
    r"(?:password|passwd|secret|token|authorization|api[_-]?key)[\"']?\s*[=: ]\s*[^\s,]+|"
    r"SCRAM-SHA-256\$|PGPASSWORD|DATABASE_URL", re.I)


def write_json(path, data):
    with path.open("x") as stream:
        json.dump(data, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def text_file(path):
    return Path(path).read_text().strip()


def builders():
    active = []
    for name in Path("/proc").glob("[0-9]*/comm"):
        try:
            command = text_file(name)
        except (FileNotFoundError, ProcessLookupError):
            continue
        if command in BUILDERS:
            active.append([int(name.parent.name), command])
    return active


def filesystem(path):
    return json.loads(subprocess.check_output([
        "findmnt", "--json", "--target", str(path), "--output", "SOURCE,TARGET,FSTYPE,MAJ:MIN",
    ], text=True))["filesystems"][0]


def sample(cpus):
    counters = {}
    for line in text_file("/proc/stat").splitlines():
        name, *values = line.split()
        if name in cpus:
            counters[name] = [int(value) for value in values[:8]]
    return {"epoch_us": time.time_ns() // 1000, "monotonic_ns": time.monotonic_ns(), "cpu": counters,
            "diskstats": text_file("/proc/diskstats").splitlines(),
            "io_pressure": text_file("/proc/pressure/io"), "loadavg": text_file("/proc/loadavg"),
            "builders": builders()}


def run(raw, config, args):
    specification = config["engines"][args.engine]
    cpus = {f"cpu{cpu}" for cpu in config["cpus"]}
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("native comparison requires x86_64 Linux")
    allowed = os.sched_getaffinity(0)
    if len(allowed) != 4 or not set(config["cpus"]) <= allowed:
        raise RuntimeError("native comparison requires exactly four available logical CPUs and the inventory mask")
    mount = filesystem(specification["postgres_storage"])
    if mount["fstype"] not in {"ext4", "xfs", "btrfs", "zfs"}:
        raise RuntimeError("PostgreSQL storage is not on the configured native disk filesystem")
    write_json(raw / "host.json", {
        "kernel": platform.release(), "architecture": platform.machine(), "processor": platform.processor(),
        "cpuinfo": text_file("/proc/cpuinfo"), "affinity": config["cpus"], "available_cpus": sorted(allowed),
        "pg_mount": mount, "boot_id": text_file("/proc/sys/kernel/random/boot_id"),
        "os_release": text_file("/etc/os-release"), "meminfo": text_file("/proc/meminfo"),
    })
    # Keep the recorder on a spare CPU. The complete adapter subtree inherits
    # exactly the same trial mask, including any PostgreSQL it starts.
    os.sched_setaffinity(0, {min(allowed - set(config["cpus"]))})
    cmd = ["taskset", "-c", ",".join(map(str, config["cpus"])), *specification["argv"],
           args.profile, str(raw / "adapter")]
    if user := specification.get("run_as"):
        account = pwd.getpwnam(user)
        os.chown(raw / "adapter", account.pw_uid, account.pw_gid)
        cmd = ["runuser", "-u", user, "--", *cmd]
    phase = "diagnostic" if args.phase == "control" else args.phase
    environment = dict(os.environ, DURABLE_JOBS_PHASE=phase)
    point = sample(cpus)
    if point["builders"]:
        write_json(raw / "busy.json", point)
        raise RuntimeError("setup build still running")
    with (raw / "host-samples.jsonl").open("x", buffering=1) as samples, (raw / "adapter.log").open("xb") as log:
        samples.write(json.dumps(point) + "\n")
        process = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT,
                                   env=environment, start_new_session=True)
        deadline = time.monotonic() + 900
        try:
            while True:
                try:
                    code = process.wait(timeout=0.20)
                except subprocess.TimeoutExpired:
                    if time.monotonic() >= deadline:
                        raise TimeoutError("adapter exceeded 15 minutes") from None
                else:
                    samples.write(json.dumps(sample(cpus)) + "\n")
                    return code
                samples.write(json.dumps(sample(cpus)) + "\n")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)


def redact_json(value):
    if isinstance(value, dict):
        return {key: redact_json(item) for key, item in value.items() if not PRIVATE_NAME.search(key)}
    if isinstance(value, list):
        return [redact_json(item) for item in value]
    if isinstance(value, str) and PRIVATE_VALUE.search(value):
        return "[redacted]"
    return value


def redact_line(line):
    try:
        document = json.loads(line)
    except (ValueError, RecursionError):
        return "[redacted]\n" if PRIVATE_VALUE.search(line) else line
    return json.dumps(redact_json(document), separators=(",", ":")) + "\n"


def safe_bytes(path):
    value = path.read_text()
    if re.search(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----", value):
        return b"[redacted private-key material]\n"
    if path.suffix == ".json":
        try:
            document = json.loads(value)
        except (ValueError, RecursionError):
            pass
        else:
            return (json.dumps(redact_json(document), separators=(",", ":")) + "\n").encode()
    if path.suffix == ".jsonl":
        return "".join(map(redact_line, value.splitlines(keepends=True))).encode()
    return "".join("[redacted]\n" if PRIVATE_VALUE.search(line) else line for line in value.splitlines(keepends=True)).encode()


def export(raw, config):
    if not raw.is_dir() or not (raw / "status.json").is_file():
        raise RuntimeError("arm is still running or has no durable exit status")
    engine = json.loads((raw / "invocation.json").read_text())["engine"]
    allow = ["ledger.json", *config["engines"][engine]["evidence_globs"]]
    with gzip.GzipFile(fileobj=sys.stdout.buffer, mode="wb", mtime=0) as compressed, \
            tarfile.open(fileobj=compressed, mode="w|") as archive:
        for current, dirs, files in os.walk(raw):
            dirs[:] = sorted(name for name in dirs if not PRIVATE_NAME.search(name)
                             and name not in {"pgdata", "scratch", "target", ".git"})
            for name in sorted(files):
                path = Path(current, name)
                relative = path.relative_to(raw)
                if (path.suffix not in SAFE_EXTENSIONS or PRIVATE_NAME.search(name) or path.is_symlink()
                        or not path.is_file()):
                    continue
                if relative.parts[0] == "adapter" and not any(
                    fnmatchcase(relative.relative_to("adapter").as_posix(), pattern) for pattern in allow
                ):
                    continue
                try:
                    data = safe_bytes(path)
                except UnicodeError:
                    continue
                member = tarfile.TarInfo(relative.as_posix())
                member.size = len(data)
                member.mode = 0o644
                archive.addfile(member, io.BytesIO(data))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("run", "export"))
    parser.add_argument("attempt")
    parser.add_argument("--engine", choices=("mesh", "oban", "go", "rust"))
    parser.add_argument("--profile", choices=("noop", "slow", "sparse", "short", "large"))
    parser.add_argument("--phase", choices=("warmup", "control", "measured", "diagnostic"))
    args = parser.parse_args()
    if not re.fullmatch(r"[a-f0-9]{32}", args.attempt):
        parser.error("invalid attempt ID")
    os.umask(0o077)
    raw = HERE / "raw" / args.attempt
    config = json.loads((HERE / "inventory.json").read_text())
    if args.action == "export":
        export(raw, config)
        return
    raw.mkdir(parents=True)
    # Allow an adapter's unprivileged user to reach its own evidence directory.
    # All siblings and files remain private and parent directories cannot be listed.
    for directory in (ROOT, HERE, raw.parent, raw):
        directory.chmod(0o711)
    (raw / "adapter").mkdir()
    write_json(raw / "invocation.json", vars(args))
    code = 1
    try:
        with (ROOT / "active-arm.lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            code = run(raw, config, args)
    except (OSError, RuntimeError, ValueError) as exc:
        write_json(raw / "error.json", {"type": type(exc).__name__, "error": str(exc)})
    finally:
        write_json(raw / "status.json", {"exit_code": code, "finished_us": time.time_ns() // 1000})
    sys.exit(code)


if __name__ == "__main__":
    main()
