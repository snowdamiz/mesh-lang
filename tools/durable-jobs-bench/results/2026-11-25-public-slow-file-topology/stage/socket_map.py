"""Loopback socket attribution from the archived V1 topology adapter."""
import os
from pathlib import Path
import re


def established_clients(table, server_port):
    """Map kernel socket inode to its client port for the fixture's loopback DB."""
    peer = f"0100007F:{server_port:04X}"
    result = {}
    for line in table.splitlines()[1:]:
        fields = line.split()
        if len(fields) > 9 and fields[2] == peer and fields[3] == "01":
            result[fields[9]] = int(fields[1].split(":")[1], 16)
    return result


def runner_backend_map(pids, postgres_port, sessions):
    ports = {item["client_port"]: item["backend"] for item in sessions if item["role"] == "compiled_caller"}
    clients = established_clients(Path("/proc/net/tcp").read_text(), postgres_port)
    mapping = {}
    for pid in pids:
        out = {}
        for fd in Path(f"/proc/{pid}/fd").iterdir():
            try:
                target = os.readlink(fd)
            except FileNotFoundError:
                continue
            match = re.fullmatch(r"socket:\[(\d+)\]", target)
            if match and (port := clients.get(match[1])) in ports:
                out[port] = {"client_port": port, "backend": ports[port]}
        mapping[str(pid)] = sorted(out.values(), key=lambda entry: entry["backend"])
    return mapping
