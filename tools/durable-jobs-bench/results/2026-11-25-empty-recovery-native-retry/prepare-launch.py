"""Bind the pinned product launcher to the staged observer."""
from pathlib import Path
import sys

from contract import rewrite


def main():
    old, phase = sys.argv[1:]
    old = Path(old)
    output = Path(__file__).resolve().with_name("native-run")
    output.write_text(rewrite(old.read_text(), old.parents[5], phase))


if __name__ == "__main__":
    main()
