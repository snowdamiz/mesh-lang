"""Check the Oban emulator and resolver before timing each arm."""

import json
import os
from pathlib import Path
import shlex


def report_from(output):
    report, = [json.loads(line.removeprefix("BEAM_CONFIG "))
               for line in output.splitlines() if line.startswith("BEAM_CONFIG ")]
    return report


def verify_report(report, config, role):
    flags = config["oban_erl_flags"]
    inetrc = config["oban_erl_inetrc"]
    tokens = shlex.split(flags)
    options = dict(zip(tokens[::2], tokens[1::2], strict=True))
    schedulers, schedulers_online = map(int, options["+S"].split(":"))
    dirty_cpu, dirty_cpu_online = map(int, options["+SDcpu"].split(":"))
    expected = dict(role=role, erl_flags=flags, erl_inetrc=inetrc,
                    flag_argv={flag: [value] for flag, value in options.items()},
                    inetrc_sha256=config["hashes"]["proof/oban.inetrc"] if inetrc else None,
                    loopback_addresses=["127.0.0.1"], schedulers=schedulers,
                    schedulers_online=schedulers_online, dirty_cpu_schedulers=dirty_cpu,
                    dirty_cpu_schedulers_online=dirty_cpu_online,
                    dirty_io_schedulers=int(options["+SDio"]), async_threads=int(options["+A"]))
    assert all(report.get(key) == value for key, value in expected.items()), report
    if inetrc:
        assert report["lookup"] == ["file"], report
    else:
        assert "native" in report["lookup"], report


def live_report(report, config, role, pid, table, descendants):
    verify_report(report, config, role)
    assert report["pid"] == pid and pid in descendants, report
    processes = []
    for child in sorted(descendants):
        proc = Path(f"/proc/{child}")
        processes.append({"pid": child, "ppid": table[child][0],
                          "comm": (proc / "comm").read_text().strip(),
                          "exe": os.readlink(proc / "exe")})
    allowed = {b"ERL_FLAGS", b"ERL_INETRC"}
    env = {key.decode(): value.decode() for key, _, value in (
        entry.partition(b"=") for entry in Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
    ) if key in allowed}
    expected = {"ERL_FLAGS": report["erl_flags"]}
    if report["erl_inetrc"]:
        expected["ERL_INETRC"] = report["erl_inetrc"]
    assert env == expected, env
    return dict(report, live_environment=env, processes=processes)
