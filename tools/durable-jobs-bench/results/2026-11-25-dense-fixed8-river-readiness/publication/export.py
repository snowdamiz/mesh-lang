"""Export the committed release and the exact offline replay dependencies."""

import argparse
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parents[1]
RESULTS = HERE.parent
SUITE = RESULTS.parent
ROOT = SUITE.parents[1]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=ROOT)


def paths(commit):
    scope = HERE.relative_to(ROOT)
    factorial = scope.with_name("2026-11-25-dense-fixed8-logical-factorial")
    dense = scope.with_name("2026-11-23-dense-matched-topology")
    equal = dense / "evidence/equal"
    exact = {
        "scripts/jobs-parity.py",
        "scripts/jobs-parity-profiles.json",
        str(factorial / "PEER-RESULTS.md"),
        str(equal / "series.json"),
        str(equal / "controller-release.json"),
        str(equal / "staging/river-2x32/inventory.json"),
        str(dense / "river-build.json"),
    }
    for packet in (scope / "controller-packet.json", factorial / "controller-packet-retry.json"):
        pins = json.loads(git("show", f"{commit}:{packet}"))["local_sha256"]
        exact.update(name for name in pins if not name.startswith("target/"))
    helpers = {str(scope.with_name(name)) + "/" for name in (
        "2026-11-23-dense-matched-topology",
        "2026-11-24-dense-product-d2-transfer",
        "2026-11-24-dense-two-runner-checkout",
        "2026-11-25-dense-fixed8-logical-factorial",
        "2026-11-25-dense-grouped-refill",
        "2026-11-25-dense-live-grant-plans",
        "2026-11-25-dense-logical-grant-key-native",
    )}
    suite = SUITE.relative_to(ROOT)
    files = git("ls-tree", "-r", "--name-only", "-z", commit, "--", suite, "scripts").decode().split("\0")
    required = set(files)
    assert exact <= required, sorted(exact - required)
    # The original released River replay needs the five measured archives and their requests.
    river = "/measured-river-2x32/measured-go/"
    original = {name for name in files if name.startswith(str(equal / "blocks") + "/") and river in name
                and Path(name).name in {"raw.tar.gz", "request.json"}}
    assert sum(name.endswith("/raw.tar.gz") for name in original) == 5
    python = {name for name in files if name.endswith(".py") and (
        Path(name).parent == suite or name.startswith("scripts/lib/") or name.startswith(tuple(sorted(helpers))))}
    return sorted(exact | original | python | {str(scope), str(factorial / "evidence/native-retry")})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("commit")
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    commit = git("rev-parse", "--verify", f"{args.commit}^{{commit}}").decode().strip()
    assert args.output.suffixes[-2:] == [".tar", ".gz"] and not args.output.exists()
    subprocess.run(["git", "archive", "--format=tar.gz", f"--output={args.output.resolve()}",
                    commit, *paths(commit)], cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
