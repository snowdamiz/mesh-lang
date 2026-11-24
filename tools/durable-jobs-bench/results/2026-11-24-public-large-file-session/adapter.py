"""Generate the public adapter variant while keeping its trial loop and clock."""

from pathlib import Path
import shutil
import sys

from build import replace_once


def backend(source):
    original = '    scope = dict(original.__globals__, process_stamp=process_stamp)'
    addition = '''    source = source.replace(
        '    output = evidence / "broker-output.log"',
        '    env["JOBS_PARITY_FIXTURE_DIR"] = str(cluster.scratch.resolve())\\n'
        '    output = evidence / "broker-output.log"',
    )
    launch = '[*self.command, "session"]'
    assert source.count(launch) == 1
    source = source.replace(launch, '[str(self.session_executable), "session"]')
    inline = '            response = request({"verb": "enqueue", "batch": batch})'
    read = '            batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)'
    assert source.count(read + "\\n" + inline) == 1
    source = source.replace(read + "\\n" + inline, """
            if os.environ["PRODUCT_SESSION_MODE"] == "file":
                fixture_dir = cluster.scratch.resolve()
                if not path.is_absolute() or path.parent.resolve() != fixture_dir or path.is_symlink():
                    raise ValueError("expected an absolute trusted batch path in private trial scratch")
                response = request({"verb": "enqueue_file", "batch": str(fixture_dir / path.name),
                                    "request_id": path.stem})
            else:
                batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)
                response = request({"verb": "enqueue", "batch": batch})
""".strip("\\n"))
''' + original
    source = replace_once(source, original, addition)
    source = replace_once(source, "self.broker_process = (broker, process_stamp(broker))",
                          "self.broker_process = (broker, process_stamp(broker) if sys.platform == 'linux' else None)")
    source = replace_once(source, "    @property\n    def executable_sha(self):",
                          '''    @property
    def session_executable(self):
        return Path(os.environ["PRODUCT_SESSION_EXECUTABLE"])

    @property
    def session_sha(self):
        return os.environ["PRODUCT_SESSION_SHA256"]

    @property
    def executable_sha(self):''')
    return replace_once(source,
        '''            launches = [observe(broker, executable, "session", self.executable_sha,
                                pinned_file(executable, self.executable_sha), start, cpus)]''',
        '''            session_executable = self.session_executable
            launches = [observe(broker, session_executable, "session", self.session_sha,
                                pinned_file(session_executable, self.session_sha), start, cpus)]''')


def stage(public, repaired, out):
    out.mkdir()
    for name in ("identity.py", "product_arm.py"):
        shutil.copy2(public / name, out / name)
    path = out / "backend.py"
    path.write_text(backend(repaired.read_text()))
    compile(path.read_bytes(), path, "exec")
    path = out / "native-run"
    text = (public / path.name).read_text()
    text = replace_once(text, 'case "$2" in short|noop|sparse|slow|large) ;; *) exit 2 ;; esac',
                        '''[[ "$2" == large && "$mode" == product-bulk && "${PRODUCT_TOPOLOGY:?}" == stock ]]
case "${PRODUCT_SESSION_MODE:?}" in inline|file) ;; *) exit 2 ;; esac
[[ "${PRODUCT_SESSION_EXECUTABLE:?}" == /* && ! -L "$PRODUCT_SESSION_EXECUTABLE" ]]
[[ "${PRODUCT_SESSION_SHA256:?}" =~ ^[a-f0-9]{64}$ ]]
printf '%s  %s\\n' "$PRODUCT_SESSION_SHA256" "$PRODUCT_SESSION_EXECUTABLE" | sha256sum -c >&2''')
    anchor = '    "PRODUCT_TOPOLOGY=${PRODUCT_TOPOLOGY:?}" "PRODUCT_STORAGE=${PRODUCT_STORAGE:?}"'
    text = replace_once(text, anchor, anchor + ''' \\
    "PRODUCT_SESSION_MODE=$PRODUCT_SESSION_MODE" "PRODUCT_SESSION_EXECUTABLE=$PRODUCT_SESSION_EXECUTABLE" \\
    "PRODUCT_SESSION_SHA256=$PRODUCT_SESSION_SHA256"''')
    path.write_text(text)
    path.chmod(0o755)


if __name__ == "__main__":
    stage(*(Path(value).resolve() for value in sys.argv[1:]))
