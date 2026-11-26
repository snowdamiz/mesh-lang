"""Bounded serial transport for the disposable Jobs comparison guest."""

import base64
import gzip
import hashlib
import io
import json
import re
import shutil
import sys
import tarfile
from pathlib import Path

BEGIN = b"MESH_JOBS_COMPARISON_BEGIN"
END = b"MESH_JOBS_COMPARISON_END"
EXIT = b"LINUX_JOBS_PG_SMOKE_EXIT"
MAX_COMPRESSED = 2 * 1024**2
MAX_EXPANDED = 32 * 1024**2
MAX_SERIAL = 8 * 1024**2
MAX_LOG = 96 * 1024
CHUNK = 144


def read_log(path):
    with path.open("rb") as stream:
        size = stream.seek(0, 2)
        stream.seek(0)
        if size <= MAX_LOG:
            return stream.read(MAX_LOG)
        first = stream.read(MAX_LOG // 2)
        stream.seek(-MAX_LOG // 2, 2)
        gap = f"\n[omitted {size - MAX_LOG} bytes]\n".encode()
        return first + gap + stream.read(MAX_LOG // 2)


def emit(evidence, archive, password, serial):
    total = 0
    with tarfile.open(archive, "w:gz", compresslevel=3) as output:
        for path in sorted(evidence.rglob("*")):
            if not path.is_file():
                continue
            is_log = path.suffix == ".log"
            if not is_log and path.stat().st_size > MAX_EXPANDED - 1024**2 - total:
                raise ValueError("expanded proof exceeds the serial budget")
            data = read_log(path) if is_log else path.read_bytes()
            if password:
                data = data.replace(password.encode(), b"[local-db-password]")
            total += len(data)
            if total > MAX_EXPANDED - 1024**2:
                raise ValueError("expanded proof exceeds the serial budget")
            entry = tarfile.TarInfo(path.relative_to(evidence).as_posix())
            entry.mode = 0o600
            entry.size = len(data)
            output.addfile(entry, io.BytesIO(data))
    size = archive.stat().st_size
    if size > MAX_COMPRESSED:
        raise ValueError("compressed proof exceeds the serial budget")
    with archive.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest().encode()
        stream.seek(0)
        serial.write(BEGIN + b" %d " % size + digest + b"\n")
        for chunk in iter(lambda: stream.read(CHUNK), b""):
            serial.write(base64.b64encode(chunk) + b"\n")
        serial.write(END + b"\n")
        serial.flush()


def read_serial(serial):
    data = bytearray()
    expected_size = None
    expected_sha = None
    ended = False
    guest_exit = None
    total = 0
    while line := serial.readline(16385):
        total += len(line)
        if len(line) > 16384 or total > MAX_SERIAL:
            raise ValueError("serial input exceeds the read budget")
        line = line.rstrip(b"\r\n")
        if line.startswith(BEGIN):
            if expected_size is not None:
                raise ValueError("duplicate guest proof header")
            header = re.fullmatch(BEGIN + rb" ([1-9][0-9]{0,7}) ([0-9a-f]{64})", line)
            if not header:
                raise ValueError("malformed guest proof header")
            expected_size = int(header[1])
            expected_sha = header[2].decode()
            if expected_size > MAX_COMPRESSED:
                raise ValueError("compressed proof exceeds the read budget")
        elif line == END:
            if expected_size is None or ended:
                raise ValueError("unexpected guest proof footer")
            ended = True
        elif expected_size is not None and not ended:
            if not 0 < len(line) <= CHUNK * 4 // 3:
                raise ValueError("invalid guest proof chunk length")
            data.extend(base64.b64decode(line, validate=True))
            if len(data) > expected_size:
                raise ValueError("guest proof exceeds its declared size")
        elif line.startswith(EXIT):
            marker = re.fullmatch(EXIT + rb" ([0-9]{1,3})", line)
            if not ended or not marker or guest_exit is not None:
                raise ValueError("malformed or unexpected guest exit marker")
            guest_exit = int(marker[1])
    if not ended or guest_exit is None or len(data) != expected_size:
        raise ValueError("incomplete guest proof or missing guest exit marker")
    if hashlib.sha256(data).hexdigest() != expected_sha:
        raise ValueError("guest proof checksum mismatch")
    return bytes(data), guest_exit


def unpack(data, destination):
    with gzip.GzipFile(fileobj=io.BytesIO(data)) as compressed:
        plain = compressed.read(MAX_EXPANDED + 1)
    if len(plain) > MAX_EXPANDED:
        raise ValueError("expanded proof exceeds the extraction budget")
    with tarfile.open(fileobj=io.BytesIO(plain), mode="r:") as archive:
        for index, member in enumerate(archive):
            parts = member.name.split("/")
            safe_name = all(re.fullmatch(r"[a-zA-Z0-9_-][a-zA-Z0-9_.-]*", part) for part in parts)
            if (index >= 256 or member.type not in (tarfile.REGTYPE, tarfile.AREGTYPE)
                    or member.sparse is not None or not safe_name
                    or not 0 <= member.size <= len(plain) - member.offset_data):
                raise ValueError(f"unsupported proof archive member {member.name!r}")
            target = destination.joinpath(*parts)
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, target.open("xb") as output:
                shutil.copyfileobj(source, output)


def extract(serial_path, output_name, host_context=None):
    if not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_.-]{0,79}", output_name):
        raise ValueError("output must be a new directory name inside the adapter directory")
    output = Path(__file__).resolve().parent / output_name
    with serial_path.open("rb") as serial:
        data, guest_exit = read_serial(serial)
    context = None
    if host_context is not None:
        with host_context.open("rb") as stream:
            raw_context = stream.read(65537)
        if len(raw_context) > 65536:
            raise ValueError("host context exceeds 64 KiB")
        context = json.loads(raw_context)
        if type(context) is not dict:
            raise ValueError("host context must be a JSON object")
    output.mkdir(mode=0o700)
    try:
        destination = output / "evidence"
        unpack(data, destination)
        result = json.loads((destination / "result.json").read_text())
        if (result.get("status"), guest_exit) not in {("complete", 0), ("failed", 1)}:
            raise ValueError("guest result and PID 1 exit marker disagree")
        (output / "proof.tar.gz").write_bytes(data)
        (output / "host.json").write_text(json.dumps({
            "guest_exit": guest_exit,
            "proof_sha256": hashlib.sha256(data).hexdigest(),
            "host_context": context,
        }, indent=2, allow_nan=False) + "\n")
    except BaseException:
        shutil.rmtree(output)
        raise
    print(output, file=sys.stdout)
    return guest_exit
