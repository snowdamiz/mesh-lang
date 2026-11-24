"""Replay the integrated proof with the established AMD64 linker and proof-only PostgreSQL copy."""

from pathlib import Path
import runpy


if __name__ == "__main__":
    replay = Path(__file__).resolve().parent
    launch = runpy.run_path(str(replay.parent / "correctness.py"))["main"]
    launch.__globals__.update(NAME="set-based-enqueue-product-native-link", REPLAY=replay)
    launch()
