"""Render the exact public launcher with the trial adapter."""

from pathlib import Path
import sys

from launcher import rewrite
from roster import SESSION_COLUMNS


def replace(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new)


def launcher(source, root, lane, module):
    rendered = rewrite(source, root, lane, module)
    entry = f'"$here/observe.py" {lane} {module} "$@"'
    assert rendered.count(entry) == 2
    return rendered.replace(entry, f'"$here/stock_arm.py" {lane} {module} "$@"')


def observer(text):
    # The pinned observer supplies the plan, statement and idle-frame collection.
    begin, end = text.index("def ports("), text.index("def scoped_frame(")
    text = text[:begin] + text[end:]
    for old, new in (
        ("import re\n", ""),
        ("SELECT p.pid, p.usename, p.client_addr, p.client_port, p.state, ssl.ssl",
         "SELECT " + SESSION_COLUMNS),
        ('{pid: ports(pid, self.port) for pid in self.runner_pids}', 'self.bound_runner_ports'),
        ("    def capture_runners(self, processes, logs):\n"
         "        super().capture_runners(processes, logs)\n"
         "        self.cluster.runner_pids = [child.pid for child in processes]\n", ""),
        ('== "product-d2"', '== "product-bulk"'),
        ('== "two-runner"', '== "stock"'),
        ("cluster.sql(\"mesh\", role_sql(MODULE))", """cluster.sql("mesh", role_sql(MODULE, sample_rate="1") + "\\n" + "\\n".join(
                f"ALTER ROLE {ROLE} IN DATABASE mesh SET {key} = '{value}';"
                for key, value in (("log_lock_waits", "on"), ("deadlock_timeout", "1ms"),
                                   ("log_parameter_max_length_on_error", "0"))))"""),
    ):
        text = replace(text, old, new)
    return text


if __name__ == "__main__":
    stage, old = map(Path, sys.argv[1:3])
    path = stage / "observe.py"
    path.write_text(observer(path.read_text()))
    (stage / "native-run").write_text(launcher(old.read_text(), old.parents[5], *sys.argv[3:]))
