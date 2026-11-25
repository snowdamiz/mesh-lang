"""Inserted into the frozen observer. Run before the isolated postmaster starts."""
import fcntl
from tempfile import TemporaryDirectory

WAL_MOUNT = {"source": "/dev/sda", "target": "/", "fstype": "ext4", "maj:min": "8:0"}
UNSAFE_OPTIONS = {"ro", "nobarrier", "barrier=0", "noload", "norecovery"}


def probe_support(data):
    need(sys.platform == "linux" and hasattr(os, "O_DSYNC") and os.O_DSYNC > 0
         and os.O_DSYNC != os.O_SYNC and hasattr(os, "fdatasync"),
         "requires distinct Linux O_DSYNC and fdatasync support")
    directory = (data / "pg_wal").resolve(strict=True)
    raw = subprocess.check_output(
        ["findmnt", "--json", "--first-only", "--target", str(directory),
         "--output", "SOURCE,TARGET,FSTYPE,MAJ:MIN,OPTIONS"], text=True, timeout=10,
    )
    mount, = json.loads(raw)["filesystems"]
    device = directory.stat().st_dev
    dev_number = f"{os.major(device)}:{os.minor(device)}"
    need(all(mount.get(key) == value for key, value in WAL_MOUNT.items())
         and dev_number == WAL_MOUNT["maj:min"]
         and not UNSAFE_OPTIONS.intersection(mount["options"].split(",")),
         "isolated pg_wal is outside the pinned writable ext4 mount")

    payload = bytes(range(256)) * 32
    files = {}
    # Exercise both methods in every arm before normal setup and warmup.
    with TemporaryDirectory(prefix=".sync-method-", dir=directory) as scratch:
        for method in ("fdatasync", "open_datasync"):
            path = Path(scratch) / method
            sync_flag = os.O_DSYNC if method == "open_datasync" else 0
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | sync_flag, 0o600)
            try:
                flags = fcntl.fcntl(fd, fcntl.F_GETFL)
                need((flags & os.O_SYNC) == sync_flag, "kernel did not retain the requested WAL open flags")
                need(os.pwrite(fd, payload, 0) == len(payload), "short setup WAL probe write")
                if method == "fdatasync":
                    os.fdatasync(fd)
            finally:
                os.close(fd)
            need(path.read_bytes() == payload, "setup WAL probe readback differs")
            files[method] = dict(flags=flags, bytes=len(payload))
    return dict(platform=sys.platform, kernel=os.uname().release, wal_mount=mount, device=dev_number,
                o_dsync=os.O_DSYNC, o_sync=os.O_SYNC, files=files, selected=SETTINGS["wal_sync_method"])
