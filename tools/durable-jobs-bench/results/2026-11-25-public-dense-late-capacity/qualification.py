"""Extend the exact public fixed-window Jobs qualification with late capacity guards."""
import argparse
from hashlib import sha256
import json
from pathlib import Path
import posixpath
import re
import subprocess
import tarfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
BASE = "3c4032d06b10bc54a47290c673562fb063b47789"
PROOF = "proof/jobs-group-success/native.py"
WIDTHS = dict(baseline=2, grow=5, shrink=1, **{"global-cap": 1}, pause=0,
              **{"namespace-pause": 0}, stop=0)


def driver(source):
    entry = source / PROOF
    original = entry.read_text()
    anchor = '        "scripts/fixtures/jobs-group-success-grant.sql",'
    assert original.count(anchor) == 1
    original = original.replace(anchor, anchor + """
        "scripts/fixtures/jobs-set-based-claim-grant.sql", "tests/vectors/blind-rsa", "tests/vectors/mlkem",""")
    scope = {"__file__": str(entry), "__name__": "public_fixed8_native_proof"}
    exec(compile(original, str(entry), "exec"), scope)
    inputs = scope["inputs"]
    replace = scope["replace_once"]

    def modified_inputs():
        files = inputs()
        key = "fixture/main.mpl"
        files[key] = replace(
            files[key], b"  let ordinary_config = ",
            b'''  let late_capacity =
    if policy == "success_cohort" && Env.get("JOBS_LATE_CAPACITY", "0") == "1" do
      ",\\"late_capacity\\":true"
    else
      ""
    end
  let ordinary_config = ''',
        )
        files[key] = replace(files[key], b'<> claims <> groups <> "}"',
                             b'<> claims <> groups <> late_capacity <> "}"')
        key = "replay/native-setup.sh"
        files[key] = replace(
            files[key],
            b'    "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$source"',
            b'    JOBS_LATE_CAPACITY=1 "TMPDIR=$TMPDIR" "PRODUCT_SOURCE=$source"',
        )
        files[key] = replace(files[key], b'sha256sum "$artifact/mesh" "$artifact/mesh.json"',
                             b'''(cd "$source" && cargo test --offline --locked --release --target x86_64-unknown-linux-gnu \\
    -p mesh-rt --lib --no-run --message-format=json) > "$out/guard-build.jsonl" 2> "$out/guard-build.log"
guard_binary="$(python3 - "$out/guard-build.jsonl" <<'PYTHON'
import json, sys
artifacts = [r["executable"] for line in open(sys.argv[1]) if (r := json.loads(line))
             .get("reason") == "compiler-artifact" and r.get("executable")]
assert len(artifacts) == 1
print(artifacts[0])
PYTHON
)"
runuser -u nobody -- env -i "PATH=$PATH:$PG_BIN_DIR" LC_ALL=C.UTF-8 "TMPDIR=$TMPDIR" taskset -c 0,1 \\
    bash "$packet/replay/native-controls.sh" "$guard_binary" "$TMPDIR" > "$out/guarded.log" 2>&1
sha256sum "$artifact/mesh" "$artifact/mesh.json"''')
        files["replay/native-controls.sh"] = (HERE / "native-controls.sh").read_bytes()
        for name in (key, "replay/native-controls.sh"):
            subprocess.run(["bash", "-n"], input=files[name], check=True)
        return files

    scope["inputs"] = modified_inputs
    return scope

def include_dependencies(files, read):
    dependencies = set()
    sites = 0
    for path in sorted(files):
        if not path.startswith("source/compiler/mesh-rt/src/") or not path.endswith(".rs"):
            continue
        for name in re.findall(r'include_(?:str|bytes)!\(\s*"([^"]+)"', read(path).decode()):
            dependency = posixpath.normpath(posixpath.join(posixpath.dirname(path), name))
            assert dependency in files, (path, dependency)
            dependencies.add(dependency)
            sites += 1
    assert len({name for name in dependencies if name.startswith("source/tests/vectors/")}) == 5
    return sites, dependencies


def check(scope, output):
    packet = scope["check"](output)
    with tarfile.open(output / "source.tar.gz") as archive:
        members = {item.name: item for item in archive if item.isfile()}

        def read(path):
            return archive.extractfile(members[path]).read()

        _, dependencies = include_dependencies(members, read)
        for path in sorted(dependencies):
            repository_path = path.removeprefix("source/")
            assert read(path) == scope["show"](packet["source_sha"], repository_path)
    return packet


def extract(scope, output, extracted):
    packet = check(scope, output)
    extracted.mkdir(parents=True, exist_ok=False)
    with tarfile.open(output / "source.tar.gz") as archive:
        archive.extractall(extracted, filter="data")
    files = {p.relative_to(extracted).as_posix() for p in extracted.rglob("*") if p.is_file()}

    def read(path):
        return (extracted / path).read_bytes()

    sites, dependencies = include_dependencies(files, read)
    verified = {}
    for path in sorted(dependencies):
        content = read(path)
        assert content == scope["show"](packet["source_sha"], path.removeprefix("source/"))
        verified[path] = sha256(content).hexdigest()
    return dict(archive_sha256=packet["archive_sha256"], source_sha=packet["source_sha"],
                include_sites=sites, dependencies=verified)


def replay(source, output, release, raw):
    scope = driver(source)
    check(scope, output)
    sealed = scope["replay"](output, release, raw)
    with tarfile.open(raw) as archive:
        manifest = archive.extractfile("./mesh.json").read()
        guarded = archive.extractfile("./guarded.log").read()
    for mode, width in WIDTHS.items():
        assert guarded.count(f"late_capacity_case: {mode}, first replacement grant width {width}\n".encode()) == 1
    assert guarded.count(b"test result: ok. 1 passed; 0 failed") == len(WIDTHS)
    return dict(sealed, manifest=json.loads(manifest), manifest_sha256=sha256(manifest).hexdigest(),
                guarded_sha256=sha256(guarded).hexdigest(), guarded_widths=WIDTHS)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("pack", "check", "extract", "replay"))
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--extracted", type=Path)
    parser.add_argument("--release", type=Path)
    parser.add_argument("--raw", type=Path)
    args = parser.parse_args()
    source, output = args.source.resolve(strict=True), args.output.resolve()
    scope = driver(source)
    if args.command == "pack":
        head = scope["git"]("rev-parse", "HEAD").decode().strip()
        assert scope["git"]("rev-parse", "HEAD^").decode().strip() == BASE
        scope["pack"](head, output)
        check(scope, output)
    elif args.command == "check":
        print(json.dumps(check(scope, output), indent=2, sort_keys=True))
    elif args.command == "extract":
        print(json.dumps(extract(scope, output, args.extracted), indent=2, sort_keys=True))
    else:
        print(json.dumps(replay(source, output, args.release, args.raw), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
