"""Seal the candidate passport after its local correctness and offline checks."""
import sys

import controller as c


def seal():
    history = c.check(require_request=False)
    c.require(history == c.run.read_json(c.PACKET / "historical-fingerprints.json"), "historical inputs changed")
    paths = [c.BUNDLE, c.PACKET / "candidate.patch", c.PACKET / "README.md",
             c.PACKET / "historical-fingerprints.json", c.PACKET / "evidence/local-gates.json",
             c.PACKET / "semantics/main.mpl", c.DENSE / "equal.json", c.DENSE / "evidence/equal/replay.json",
             c.DENSE / "qualification.py", c.TOPOLOGY / "arm.py", c.TOPOLOGY / "replay.py",
             c.V1 / "ordinary-trusted-v1.tar.gz",
             c.ROOT / "target/ordinary-trusted-mesh-queue/fixture/main.mpl"]
    paths.extend(c.HERE.glob("*.py"))
    paths.extend(c.HERE.glob("*.sh"))
    paths.extend(c.SUITE / name for name in ("interleave.py", "run.py", "worker.py", "verify.py"))
    paths.extend(c.ROOT / name for name in ("scripts/jobs-parity.py", "scripts/jobs-parity-profiles.json"))
    paths.extend(path for group in c.historical_paths().values() for path in group)
    command = f"python3 -B {c.HERE.relative_to(c.ROOT) / 'controller.py'}"
    prefix = f"cd {c.ROOT} && {command}"
    request = dict(
        id=c.ID, controller="linode-native-stepchange-controller-v1", state="ready", ready=True,
        operator_id="trusted-handler-app-pool-v1", kind="Controller-owned native SETUP and exclusive build-free TIMING",
        requested_at=c.run.utc_now(), source_sha=c.SOURCE,
        candidate_patch_sha256=c.run.digest(c.PACKET / "candidate.patch"),
        source_archive=str(c.BUNDLE.relative_to(c.ROOT)), source_archive_sha256=c.run.digest(c.BUNDLE),
        staging_path=str(c.PACKET), evidence_destination=str(c.EVIDENCE),
        command=f"{prefix} setup", setup_command=f"{prefix} setup",
        verify_setup_command=f"{prefix} verify-setup", timing_command=f"{prefix} timing 1 2",
        remaining_timing_command=f"{prefix} timing 3 4 5 6", replay_command=f"{prefix} replay",
        offline_check=f"{prefix} check", expected_setup_minutes=25, requested_window_minutes=28,
        requested_timing_windows_minutes=[22, 42], cpus=[0, 1], clock_ticks=100,
        native_host=c.HOST, postgres_sha256=c.POSTGRES,
        input_sha256={profile: c.run.PROFILES[profile][3] for profile in c.GEOMETRY},
        priority="After current bookings. The sole controller chooses grant order, handles native access, and publishes releases.",
        prerequisites="Original pinned boot, kernel, disk, PG17.11 executable and frozen V1 broker/migrator. "
                      "SETUP must pass and release before TIMING. Replay groups 1 and 2 before dispatching 3 through 6.",
        scope="Isolated app-pool handoff on the exit-confirmed db5937e7 substrate. "
              "Frozen dense and short with native duplicate unobserved service, direct, service/direct with "
              "handler stamps, and the exact frozen V1 reference. Reuse the same scalar V1 broker and migrator in every arm.",
        resource_bounds=dict(cpus=[0, 1], arms=108, setups=1, one_arm_at_a_time=True, repeats_per_workload=6,
                             arm_adapter_timeout_seconds=180, cpu_steal_percent_max=1,
                             native_build_workers=4, native_build_in_timing=False,
                             profiles={name: dict(runners=row[0], slots_each=row[1], tls_app_sessions_each=row[2],
                                                  postgres_connection_cap=row[3], jobs=row[4])
                                       for name, row in c.GEOMETRY.items()}),
        decision="Judge dense on primary observer-off full-submit, first-entry-to-first-commit and drain gains "
                 "over the faster duplicate in at least 5 of 6 groups, with median drain gain >=62.4 ms. "
                 "Use observed lanes and observer tax to attribute the lookup interval.",
        group_plan=c.schedule(),
        local_sha256={str(path.relative_to(c.ROOT)): c.run.digest(path) for path in sorted(set(paths))},
    )
    c.run.write_json(c.REQUEST, request)
    c.check()
    print(f"{c.run.digest(c.REQUEST)}  {c.REQUEST.relative_to(c.ROOT)}")


if __name__ == "__main__":
    c.require(len(sys.argv) == 1, "seal takes no arguments")
    seal()
