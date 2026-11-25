"""Re-score the original B exports and check pilot staging without a controller."""
from pathlib import Path
import tempfile

import pilot
from replay import PATH, PILOT, fetch, read


def main():
    pilot.verified_dependencies()
    build, routes = pilot.history()
    historical = read("replay.json")
    qualified = dict(manifest=build, manifest_sha256=pilot.run.read_json(
        pilot.HERE / "reference-arm.json")["manifest_sha256"],
        elf_sha256=build["files"]["mesh"], packet="public-dense-fixed8-jobs-ec82fb2eefcb-setup")
    score = pilot.scorer()
    scratch = pilot.REPO / "target"
    scratch.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="late-capacity-check-", dir=scratch) as temporary:
        work = Path(temporary)
        stages = work / "staging"
        for label in ("A1", "A2", "reference", "B"):
            pilot.staging(label, qualified, stages / label)
        assert all((stages / "A1" / name).read_bytes() == (stages / "A2" / name).read_bytes()
                   for name in ("public-native-run", "public_arm.py", "inventory.json"))
        off = (stages / "A1/public-native-run").read_bytes()
        on = (stages / "B/public-native-run").read_bytes()
        assert off.replace(b"JOBS_LATE_CAPACITY=0", b"JOBS_LATE_CAPACITY=1") == on and on != off
        assert b"JOBS_LATE_CAPACITY" not in (stages / "reference/public-native-run").read_bytes()
        assert all(pilot.remote(label).parent.parent == pilot.run.REMOTE
                   for label in ("A1", "A2", "B", "reference"))
        for group in (1, 2):
            block = f"{PATH}/blocks/{group:02}-noop"
            capture = work / f"historical-B-{group}"
            capture.mkdir()
            for filename in ("request.json", "dispatch.json", "raw.tar.gz"):
                (capture / filename).write_bytes(fetch(PILOT, f"{block}/measured-B/measured-mesh/{filename}"))
            (capture / "inventory.json").write_bytes(fetch(PILOT, f"{PATH}/staging/B/inventory.json"))
            result = score(capture, "measured", pilot.LANE, pilot.run.read_json(capture / "inventory.json"),
                           build, routes)
            expected = historical["groups"][str(group)]["measured-B"]
            for key in ("submit_us", "drain_us", "terminal_xids", "raw_sha256", "last_effect_us"):
                assert result[key] == expected[key], key
    print("Both exact historical B exports rescored. All three staging lanes passed offline checks.")


if __name__ == "__main__":
    main()
