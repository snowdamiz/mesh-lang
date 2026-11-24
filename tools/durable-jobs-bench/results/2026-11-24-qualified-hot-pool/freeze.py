"""Reproduce the current runtime archive and the checked/hot proof fixture."""
from io import BytesIO
import gzip
from hashlib import sha256
from pathlib import Path
import runpy
import subprocess
import sys
import tarfile

HERE = Path(__file__).resolve().parent
PREVIOUS = HERE.parent / "2026-11-24-e4156740-trusted-jobs-native"
ROOT = HERE.parents[3]
REV = "1e70f18d624a841740d3bbb3d0fd09b269257f68"


def replace_once(content, old, new):
    if content.count(old) != 1:
        raise ValueError(f"unrecognized proof input {old!r}")
    return content.replace(old, new)


def overlay(path):
    data = path.read_bytes()
    if path.name == "proof.py":
        data = replace_once(data, b"def check_grant(out, executable, mixed):",
                            b"def check_grant(out, executable, mixed, hot=False):")
        data = replace_once(data, b'port=app.port)),', b'port=app.port) + ("&mesh_pool_hot_checkout=1" if hot else "")),')
        data = replace_once(data, b"assert probes >= len(inserts), app_sql",
                            b"assert (probes < len(inserts) if hot else probes >= len(inserts)), app_sql")
        data = replace_once(data, b'        ("teardown-release",', b'''        ("short-hot-grant", lambda dest: check_grant(dest, executable, mixed=False, hot=True)),
        ("hot-ambiguous-application-write", lambda dest: handler_cut(dest, executable, fault=True, hot=True)),
        ("teardown-release",''')
    elif path.name == "semantic.py":
        data = replace_once(data, b"def handler_cut(out, executable, fault=None):",
                            b"def handler_cut(out, executable, fault=None, hot=False):")
        data = replace_once(data, b'run_env = dict(env, MESH_EFFECT_APP_URL=app_url)',
                            b'run_env = dict(env, MESH_EFFECT_APP_URL=app_url + ("&mesh_pool_hot_checkout=1" if hot else ""))')
    if path.suffix == ".py":
        compile(data, str(path), "exec")
    return data


def freeze(output):
    pack = runpy.run_path(str(PREVIOUS / "pack.py"))
    args = ["git", "archive", "--format=tar", "--prefix=source/", REV, *pack["INPUTS"]]
    with (output.open("xb") as stream, gzip.GzipFile(filename="", mode="wb", fileobj=stream, mtime=0) as compressed,
          tarfile.open(fileobj=compressed, mode="w|") as result,
          subprocess.Popen(args, cwd=ROOT, stdout=subprocess.PIPE) as git):
        with tarfile.open(fileobj=git.stdout, mode="r|") as source:
            for item in source:
                if not (item.isfile() or item.isdir()):
                    raise ValueError(item.name)
                result.addfile(item, source.extractfile(item) if item.isfile() else None)
        if git.wait():
            raise RuntimeError("git archive failed")
        for path in sorted((PREVIOUS / "overlay").rglob("*")):
            if path.suffix == ".pyc" or path.name == "__pycache__":
                raise ValueError(f"unexpected generated file {path}")
            data = overlay(path) if path.is_file() else b""
            entry = result.gettarinfo(str(path), str(path.relative_to(PREVIOUS)))
            entry.mode = 0o644 if path.is_file() else 0o755
            entry.uid, entry.gid, entry.uname, entry.gname, entry.mtime = 0, 0, "", "", 0
            entry.size = len(data)
            result.addfile(entry, BytesIO(data) if path.is_file() else None)
    return sha256(output.read_bytes()).hexdigest()


if __name__ == "__main__":
    print(freeze(Path(sys.argv[1])))
