"""Reproduce the broker output race with the native adapter's exact reader and a real child."""
import ast
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile


HERE = Path(__file__).resolve().parent
TRIAL = Path("proof/jobs-parity/linux-regression/trial.py")
RAW = (HERE.parent / "evidence/native/timing-closure-retry/dense-stock/blocks/03-noop.1795496194789458000"
       / "warmup-product/warmup-mesh/raw.tar.gz")


def poll_method(source):
    fn, = (node for node in ast.walk(ast.parse(source)) if isinstance(node, ast.FunctionDef) and node.name == "poll_line")
    return fn


def poller(source, reader, broker, errors):
    # The surrounding module starts the guest harness when imported.
    ns = dict(reader=reader, broker=broker, errors=errors)
    exec(compile(ast.Module([poll_method(source)], type_ignores=[]), str(TRIAL), "exec"), ns)
    return ns["poll_line"]


def check_exit(source, tmp, prefix, suffix, expected, *, exit_code=0):
    output = tmp / "stdout"
    errors = tmp / "stderr"
    output.write_bytes(prefix)
    child = ("import os,sys; os.read(0,1); os.write(1,bytes.fromhex(sys.argv[1])); "
             "os.write(2,b'broker exited'); sys.exit(int(sys.argv[2]))")
    with output.open("ab") as sink, errors.open("wb") as err, output.open("rb") as reader, subprocess.Popen(
        [sys.executable, "-I", "-S", "-B", "-c", child, suffix.hex(), str(exit_code)],
        stdin=subprocess.PIPE, stdout=sink, stderr=err,
    ) as process:
        class ExitDuringPoll:
            def poll(self):
                process.stdin.write(b"x")
                process.stdin.flush()
                return process.wait(timeout=5)

        try:
            value = poller(source, reader, ExitDuringPoll(), errors)()
        except RuntimeError as failure:
            assert expected is None and str(failure) == "broker exited", failure
        else:
            assert expected is not None and value == expected, (expected, value)
        assert process.returncode == exit_code


def main():
    source = (HERE / "pinned-trial.py").read_bytes()
    expected = "db8e121ab9b1f6aed2293b26c79a4fbfa8ac8aba6ff228be54b360bd32048a8e"
    assert hashlib.sha256(source).hexdigest() == expected
    assert hashlib.sha256(RAW.read_bytes()).hexdigest() == "0eb4b74c11b214f47d5f9cfe049794db4f699893a36e578ce202a0aaed782acd"
    with tarfile.open(RAW) as archive:
        assert expected in json.load(archive.extractfile("host.json"))["verified_files"].values()
        assert json.load(archive.extractfile("adapter/failure.json"))["stage"][-1][2] == "poll_line"
        assert archive.extractfile("adapter/broker-output.log").read().endswith(b"{}\n")
        assert archive.extractfile("adapter/broker-errors.log").read() == b""
    with tempfile.TemporaryDirectory(dir=HERE) as work:
        tmp = Path(work)
        trial = tmp / TRIAL
        trial.parent.mkdir(parents=True)
        trial.write_bytes(source)
        subprocess.run(["patch", "-s", "-p1", "-i", str(HERE / "final-reply.patch")], cwd=tmp, check=True)
        fixed = trial.read_bytes()
        assert ast.dump(poll_method(fixed)) == ast.dump(poll_method((HERE.parents[4] / TRIAL).read_bytes()))
        for prefix, suffix in ((b"", b"{}\n"), (b"{", b"}\n")):
            check_exit(source, tmp, prefix, suffix, None)
            check_exit(fixed, tmp, prefix, suffix, "{}")
        check_exit(fixed, tmp, b"", b"", None)
        check_exit(fixed, tmp, b"", b"{}", None)
        check_exit(fixed, tmp, b"", b"{", None, exit_code=3)
    print("Verified native failure, both exit schedules, complete replies, and refusal of absent or truncated replies.")


if __name__ == "__main__":
    main()
