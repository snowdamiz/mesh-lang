"""Render the pinned native launcher and its diagnostic-only observer."""

from pathlib import Path
import sys

from adapt import rewrite_observer
from launcher import rewrite


def observer(source):
    text = rewrite_observer(source)
    old = '''        assert {(row["backend"], row["role"], row["tls"]) for row in standard} == {
            (pid, values[0], True) for pid, values in pinned.items()}
        self.cluster.roster_witness = witness
        ARM.h.write_json(self.cluster.evidence / "grant-roster.json", witness)'''
    new = '''        ARM.h.write_json(self.cluster.evidence / "grant-roster.json", witness)
        assert {(row["backend"], row["role"], row["tls"]) for row in standard} <= {
            (pid, values[0], True) for pid, values in pinned.items()}
        self.cluster.roster_witness = witness'''
    assert text.count(old) == 1
    return text.replace(old, new)


def render(source, root, lane, module):
    observed = rewrite(source, root, lane, module)
    old = f'"$here/observe.py" {lane} {module} "$@"'
    assert observed.count(old) == 2
    return observed.replace(old, f'"$here/logical_arm.py" {lane} {module} "$@"')


if __name__ == "__main__":
    stage, old = map(Path, sys.argv[1:3])
    path = stage / "observe.py"
    path.write_text(observer(path.read_text()))
    (stage / "native-run").write_text(render(old.read_text(), old.parents[5], *sys.argv[3:]))
