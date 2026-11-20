"""Run the Windows installer proof with a real pseudoconsole."""

import re
import select
import time

from winpty import Backend, PtyProcess


def main():
    offer = "Set up the Mesh skill for your coding tools? [y/N]"
    answers = iter([
        (offer, "n"),
        (offer, "y"),
        ("Tools (Enter to cancel):", "1"),
        ("Scope (Enter to cancel):", "1"),
        ("Install the Mesh skill? [y/N]", "y"),
    ])
    terminal = PtyProcess.spawn(
        ["pwsh", "-NoLogo", "-NoProfile", "-File", "scripts/verify-m034-s03.ps1", "-InteractiveSkills"],
        dimensions=(40, 240),
        backend=Backend.ConPTY,
    )
    escape = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
    deadline = time.monotonic() + 600
    pending = next(answers, None)
    buffer = ""
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select([terminal], [], [], 0.25)
            if not ready:
                if not terminal.isalive():
                    break
                continue
            try:
                chunk = terminal.read(4096)
            except EOFError:
                break
            print(chunk, end="", flush=True)
            if pending is None:
                continue
            buffer = escape.sub("", buffer + chunk)[-4096:]
            prompt, answer = pending
            if prompt in buffer:
                terminal.write(answer + "\r")
                buffer = buffer.split(prompt, 1)[1]
                pending = next(answers, None)
        else:
            raise TimeoutError(f"Windows installer proof timed out waiting for {pending!r}")

        if pending is not None:
            raise RuntimeError(f"Windows installer proof ended before prompt {pending[0]!r}")
        while terminal.isalive() and time.monotonic() < deadline:
            time.sleep(0.05)
        if terminal.isalive() or terminal.exitstatus != 0:
            raise RuntimeError(f"Windows installer proof exit status: {terminal.exitstatus}")
    finally:
        terminal.close(force=True)


if __name__ == "__main__":
    main()
