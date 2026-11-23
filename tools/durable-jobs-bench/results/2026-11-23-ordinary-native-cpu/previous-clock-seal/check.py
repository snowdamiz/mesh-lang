"""Local checks for procfs identity, phase arithmetic, and control rejection."""
from copy import deepcopy
import json
from pathlib import Path
import tempfile
from unittest import TestCase, main

from collector import capture_frame, pg_cpu
from diagnose import history, config_for_replay
from reduce import HERE, ROOT, phase_budget, phase_times, tick_interval, verdict

ARCHIVE = ROOT / "target/ordinary-trusted-mesh-queue/evidence/ordinary-trusted-v1-dense"


def proc_process(root, pid, comm, parent, argv, child_pids=()):
    path = root / str(pid)
    (path / "task" / str(pid)).mkdir(parents=True)
    (path / "task" / str(pid) / "children").write_text(" ".join(map(str, child_pids)))
    fields = ["S", str(parent)] + ["0"] * 42
    fields[11], fields[12], fields[17], fields[19] = ["16", "3", "1", str(pid + 10000)]
    (path / "stat").write_text(f"{pid} ({comm}) " + " ".join(fields) + "\n")
    (path / "cmdline").write_bytes(b"\0".join(value.encode() for value in argv) + b"\0")


def sample_frames():
    wall_offset = 1700000000000000000
    frames = []
    for index in range(51):
        age = index * 20_000_000

        def stamp(offset):
            point = age + offset
            return [point, wall_offset + point, point + 1000]

        frames.append({
            "begin": stamp(0), "end": stamp(8000),
            "host": {"begin": stamp(2000), "end": stamp(4000),
                     "ticks": {cpu: [index, 0, 0, index, 0, 0, 0, 0] for cpu in ("cpu0", "cpu1")}},
            "processes": {
                "100": {"start": 10, "parent": 1, "comm": "postgres", "user": 0, "system": 0,
                        "role": "postmaster", "cpu_ns": 0},
                "101": {"start": 11, "parent": 100, "comm": "postgres", "user": index // 3,
                        "system": 0, "role": "pg_jobs", "cpu_ns": (index * 10_000_000) // 3},
                "102": {"start": 12, "parent": 1, "comm": "mesh", "user": index // 2,
                        "system": 0, "role": "runner", "cpu_ns": index * 5_000_000},
                "103": {"start": 13, "parent": 1, "comm": "mesh", "user": 0,
                        "system": 0, "role": "broker", "cpu_ns": 0},
            },
        })
    edges = (wall_offset // 1000 + 90_000, wall_offset // 1000 + 860_000)
    registry = {"postmaster": "100", "backend_roles": {"101": "pg_jobs"},
                "runner_pids": ["102"], "broker_pid": "103"}
    return frames, edges, registry


class DiagnosticTests(TestCase):
    def test_exact_registry_and_connection_replacement(self):
        with tempfile.TemporaryDirectory(dir=HERE, prefix="proc-test-") as scratch:
            proc, adapter = (Path(scratch) / name for name in ("proc", "adapter"))
            proc.mkdir()
            adapter.mkdir()
            (proc / "stat").write_text("cpu0 1 0 1 18 0 0 0 0\ncpu1 2 0 2 16 0 0 0 0\n")
            executable = "/sealed/mesh"
            runners = list(range(11, 19))
            backends = list(range(101, 166))
            proc_process(proc, 2, "postgres", 1, ["postgres", "-D", "db"], backends)
            proc_process(proc, 3, "python", 1, ["python", "arm.py"], [*runners, 20])
            for pid in runners:
                proc_process(proc, pid, "mesh (runner) two", 3, [executable, "run"])
            proc_process(proc, 20, "mesh", 3, [executable, "session"])
            (proc / "20" / "fd").mkdir()
            (proc / "20" / "fd" / "1").symlink_to(adapter / "broker-output.log")
            masks = dict.fromkeys([2, *runners], [0, 1])
            roles = ["parity_subject"] * 32 + ["compiled_caller"] * 32 + ["parity_producer"]
            sessions = [{"backend": pid, "role": role, "tls": True}
                        for pid, role in zip(backends, roles, strict=True)]
            for row in sessions:
                proc_process(proc, row["backend"], "postgres", 2,
                             [f"postgres: {row['role']} mesh 127.0.0.1(54530) idle"])
            (adapter / "process-affinity.json").write_text(json.dumps(masks))
            (adapter / "postgres-tls-sessions.json").write_text(json.dumps(sessions))
            frame, registry = capture_frame(adapter, "mesh", [0, 1], executable, None, proc,
                                            cpu_clock=lambda _pid: 187_000_000)
            self.assertNotIn("roster_error", frame)
            self.assertEqual(registry["broker_pid"], "20")
            self.assertTrue(registry["broker_witness"]["stdout_verified"])
            self.assertEqual(len(frame["processes"]), 75)
            self.assertEqual(pg_cpu(11, proc)["comm"], "mesh (runner) two")
            moved_pid = backends[-1]
            (proc / str(moved_pid) / "cmdline").write_bytes(b"postgres: parity_subject mesh idle\0")
            frame, _ = capture_frame(adapter, "mesh", [0, 1], executable, registry, proc)
            self.assertIn("differ from the TLS census", frame["roster_error"])

    def test_conservative_bounds_and_corruption(self):
        frames, edges, registry = sample_frames()
        self.assertEqual(tick_interval([(7, 9), (3, 3), (2, 6)], 100), (0.04, 0.09))
        def reduce(test_frames, test_edges=edges):
            return phase_budget(test_frames, test_edges, 100, ["cpu0", "cpu1"], 1000, registry, {})

        budget = reduce(frames)
        self.assertEqual(budget["duration_ms"], 770.)
        self.assertAlmostEqual(budget["brackets"]["outer_edge_ms"][0], 10.)
        pg = budget["role_cpu_ms_per_job"]["pg_jobs"]
        self.assertGreater(pg["ticks_user"][1] - pg["ticks_user"][0], pg["total"][1] - pg["total"][0])
        self.assertLessEqual(pg["total"][0], 0.770 / 6)
        self.assertGreaterEqual(pg["total"][1], 0.770 / 6)
        for change, expected in (
            (lambda f: f[20]["processes"].pop("101"), "changed role"),
            (lambda f: f[20]["processes"]["101"].update(start=99999), "PID start time"),
            (lambda f: f[20]["processes"]["101"].update(cpu_ns=0), "backwards"),
            (lambda f: f[20]["end"].__setitem__(1, f[20]["end"][1] + 8_000_000), "offsets"),
            (lambda f: f[20]["host"]["ticks"]["cpu1"].__setitem__(3, 0), "host CPU counter moved backwards"),
            (lambda f: f.__delitem__(slice(16, 25)), "missed frame"),
        ):
            corrupt = deepcopy(frames)
            change(corrupt)
            with self.subTest(expected=expected), self.assertRaisesRegex(ValueError, expected):
                reduce(corrupt)
        with self.assertRaisesRegex(ValueError, "no enclosing"):
            reduce(frames[10:])

    def test_reject_perturbed_controls(self):
        rows = [{"lane": "ordinary", "order": order, "mode": mode,
                 "phase_ms": {"producer": 100., "full": drain + 100., "drain": drain},
                 "score": {"runner_cpu_ms_per_job": 1.5}}
                for order in ("forward", "reverse")
                for mode, drain in zip(("off", "on", "off"), (1000., 1010., 1020.), strict=True)]
        self.assertTrue(verdict(rows)["ordinary"]["drain_valid"])
        rows[4]["phase_ms"]["drain"] = 1100.
        self.assertFalse(verdict(rows)["ordinary"]["drain_valid"])

    def test_archive_exact_ledger_and_known_drain(self):
        data = history(ARCHIVE, config_for_replay())
        self.assertEqual(data["arms"]["ordinary"]["phase_ms"]["drain"], 2245.036)
        self.assertEqual(data["arms"]["river"]["phase_ms"]["drain"], 1046.701)
        self.assertEqual(data["arms"]["ordinary"]["score"]["ordered_independent_writes"], 1024)
        self.assertEqual(data["arms"]["ordinary"]["statement_counts"]["application"][0]["calls"], 2048)
        with self.assertRaisesRegex(ValueError, "phase clocks"):
            phase_times({"transactions": [{"submitted_us": 100, "committed_us": 99, "returned_us": 102}],
                         "jobs": [{"settled_us": 200}]})


if __name__ == "__main__":
    main()
