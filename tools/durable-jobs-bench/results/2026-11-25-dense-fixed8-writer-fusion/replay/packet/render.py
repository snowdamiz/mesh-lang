"""Keep the native launcher contract while pointing it at the staged adapter."""

from pathlib import Path
import sys


def render(source, root, lane):
    anchor = 'source="$(cd "$here/../../../../.." && pwd -P)"'
    invoke = '"$here/product_arm.py" "$@"'
    assert source.count(anchor) == 1 and source.count(invoke) == 2
    return source.replace(anchor, f'source="{root}"').replace(
        invoke, f'"$here/factorial_arm.py" {lane} "$@"')


if __name__ == "__main__":
    old = Path(sys.argv[1])
    sys.stdout.write(render(old.read_text(), old.parents[5], sys.argv[2]))
