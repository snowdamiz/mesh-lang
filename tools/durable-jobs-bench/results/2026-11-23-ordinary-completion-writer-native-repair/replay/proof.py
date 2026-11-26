"""Preserve native short scoring and additionally verify the archived launch witnesses."""
import errno
import importlib.util

from archived import ORIGINAL

spec = importlib.util.spec_from_file_location("frozen_writer_proof", ORIGINAL / "replay/proof.py")
frozen = importlib.util.module_from_spec(spec)
spec.loader.exec_module(frozen)


def check(read, mode, runner, *, native):
    score = frozen.check(read, mode, runner, native=native)
    if not native:
        return score
    split, identity, affinity, host = map(
        read, ("split-executables.json", "process-identity.json", "process-affinity.json", "host-before.json"))
    assert host["system"] == "Linux" and host["architecture"] == "x86_64"
    cpus = identity["cpus"]
    assert cpus == host["cpus"] == [0, 1]
    assert len(affinity) == 2 and all(mask == cpus for mask in affinity.values())
    launches = identity["launches"]
    assert set(launches) == {"broker", "runner"}
    assert len({identity["adapter_pid"], launches["broker"]["pid"], launches["runner"]["pid"],
                *map(int, affinity)}) == 4
    assert split["runner_pids"] == [launches["runner"]["pid"]]
    assert str(launches["runner"]["pid"]) in affinity
    for role, path, verb, expected in (
        ("broker", split["producer"], "session", frozen.BULK_ELF),
        ("runner", split["runner"], "run", runner["elf"]),
    ):
        launch = launches[role]
        assert launch["pid"] > 1 and launch["parent_pid"] == identity["adapter_pid"] > 1
        assert launch["argv"] == [path, verb] and launch["cpus"] == cpus
        assert launch["executable_sha256"] == expected
        assert launch["launch_start_ticks"] == launch["start_ticks_before"] == launch["start_ticks_after"] > 0
        if launch["method"] == "owned-launch":
            assert launch["observed_executable"] is None and launch["exe_errno"] in {errno.EACCES, errno.EPERM}
        else:
            assert launch["method"] == "proc-exe"
            assert launch["observed_executable"] == path and launch["exe_errno"] is None
    return score
