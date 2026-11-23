#!/usr/bin/env python3
"""Run the exact peer launchers with an explicit two-runner dense allocation."""

import hashlib
import importlib.util
from pathlib import Path
import sys

STOCK_LAUNCHERS = {
    "go": "e97ede452fb9df9c62fc9b12c914dbb2b5c92b2cd64165b7d11ac5e2dcc5d030",
    "oban": "95766ac87a9f04dfdd253af52a04d83438d970da7d01c6cdadfea682cebb473e",
    "rust": "05c94874455c7c48e0ca6e9204b412aa67d41096b6cf314c0d4a9f3a8fdaf0f7",
}
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
CANONICAL = 8, 64, 96, INPUT, True
OBAN_CANONICAL = 8, 64, 4, 96, 2, INPUT


def load(engine, path):
    if hashlib.sha256(path.read_bytes()).hexdigest() != STOCK_LAUNCHERS[engine]:
        raise ValueError(f"{engine}: stock launcher bytes changed")
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("dense_peer", path)
    adapter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(adapter)
    return adapter


def configure(engine, adapter, out):
    if engine == "oban":
        if adapter.PROFILES["noop"] != OBAN_CANONICAL:
            raise ValueError("stock Oban profile changed")
        adapter.PROFILES = {"noop": (2, *OBAN_CANONICAL[1:])}
        original = adapter.Subject

        class Subject(original):
            roster = {}

            def run(self, env, index, log):
                slots = int(env["JOBS_PARITY_SLOTS"])
                if slots != 32:
                    raise ValueError("Oban runner requested with wrong slots")
                proc = super().run(env, index, log)
                self.roster[proc.pid] = dict(index=index, slots=slots)
                return proc

            def capture_runners(self, processes, logs):
                super().capture_runners(processes, logs)
                if len(processes) != 2 or set(self.roster) != {proc.pid for proc in processes}:
                    raise ValueError("Oban runner roster differs")
                adapter.h.write_json(out / "runner-topology.json", self.roster)

        adapter.Subject = Subject
        return
    # This adapter process runs only the two-runner dense profile. Keep the
    # frozen input and all reconciliation checks, and bind the runner count
    # used by both raw verification and the emitted matrix ledger.
    import verify
    if verify.PROFILES["noop"] != CANONICAL:
        raise ValueError("stock peer fixture changed")
    verify.PROFILES["noop"] = (2, *CANONICAL[1:])
    if engine == "rust":
        original = adapter.parity.RunnerProcess
        roster = {}

        def launch(argv, **kwargs):
            if argv[1] != "worker" or argv[2] not in ("runner-0", "runner-1") or argv[3] != "32":
                raise ValueError("unexpected Apalis worker launch")
            proc = original(argv, **kwargs)
            roster[argv[2]] = dict(pid=proc.pid, slots=int(argv[3]))
            return proc

        adapter.parity.RunnerProcess = launch
        return roster


def main():
    engine, launcher, *args = sys.argv[1:]
    if engine not in STOCK_LAUNCHERS or len(args) < 2 or args[-2] != "noop" or "--smoke" in args:
        raise SystemExit("expected one two-runner frozen noop arm")
    path, out = Path(launcher).resolve(strict=True), Path(args[-1])
    if "--local-smoke" in args and engine != "oban":
        raise SystemExit("local smoke switch belongs to the Oban adapter")
    adapter = load(engine, path)
    roster = configure(engine, adapter, out)
    sys.argv = [str(path), *args]
    adapter.main()
    if engine == "rust":
        if set(roster) != {"runner-0", "runner-1"} or len({row["pid"] for row in roster.values()}) != 2:
            raise ValueError("Apalis runner roster differs")
        adapter.parity.write_json(out / "runner-topology.json", roster)


if __name__ == "__main__":
    main()
