"""Rebuild pinned River twice, changing only the dense runner-count literal."""

from pathlib import Path
import argparse
import json
import os
import shutil
import subprocess

from qualification import PACKET, run, require


def build(source, output):
    stock = run.read_json(PACKET / "stock.json")["inventory"]["engines"]["go"]
    originals = {name.split("/source/", 1)[1]: digest for name, digest in stock["verified_files"].items()
                 if "/source/" in name}
    for name, digest in originals.items():
        require(run.digest(source / name) == digest, f"stock River source differs: {name}")
    output.mkdir()
    src = output / "src"
    shutil.copytree(source / "tools/durable-jobs-bench/go", src)
    compile_env = dict(os.environ, CGO_ENABLED="0", GOARCH="amd64", GOAMD64="v1", GOOS="linux",
                       GOFLAGS="", GOTOOLCHAIN="local", GOWORK="off", GOPROXY="off",
                       GOCACHE=str(output / "cache"))
    require(subprocess.check_output(["go", "version"], env=compile_env, text=True).startswith("go version go1.26.4 "),
            "expected the stock River Go compiler")
    command = ["go", "build", "-mod=readonly", "-trimpath", "-buildvcs=false", "-o"]
    stock_binary, candidate = output / "river-stock-rebuilt", output / "river-2x32"
    subprocess.run([*command, str(stock_binary), "."], cwd=src, env=compile_env, check=True)
    require(run.digest(stock_binary) == stock["artifact_sha256"], "stock River did not rebuild byte-for-byte")
    path = src / "main.go"
    original = path.read_text()
    before = f'"noop":   {{8, 64, 96, true, "{INPUT}"}}'
    require(original.count(before) == 1, "exact River noop profile anchor missing")
    path.write_text(original.replace(before, before.replace("{8,", "{2,", 1), 1))
    subprocess.run([*command, str(candidate), "."], cwd=src, env=compile_env, check=True)
    darwin = output / "river-2x32-darwin"
    compile_env.update(GOOS="darwin", GOARCH="arm64")
    compile_env.pop("GOAMD64")
    subprocess.run([*command, str(darwin), "."], cwd=src, env=compile_env, check=True)
    info = subprocess.check_output(["go", "version", "-m", str(candidate)], text=True).splitlines()[1:]
    receipt = dict(stock_rebuild_sha256=run.digest(stock_binary), candidate_sha256=run.digest(candidate),
                   darwin_sha256=run.digest(darwin), stock_source=originals,
                   patched_main_sha256=run.digest(path), change=dict(before=before, after=before.replace("{8,", "{2,", 1)),
                   go_version="go1.26.4", build_info=info)
    run.write_json(output / "build-river.json", receipt)
    return receipt


if __name__ == "__main__":
    from qualification import INPUT

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    print(json.dumps(build(args.source.resolve(), args.output.resolve()), indent=2))
