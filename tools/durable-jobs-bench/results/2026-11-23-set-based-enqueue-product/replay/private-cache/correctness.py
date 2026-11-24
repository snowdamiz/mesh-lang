"""Run the same native product proof with its own offline Cargo cache."""

from pathlib import Path
import runpy


if __name__ == "__main__":
    replay = Path(__file__).resolve().parent
    launch = runpy.run_path(str(replay.parent / "correctness.py"))["main"]
    launch.__globals__.update(NAME="set-based-enqueue-product-private-cache", REPLAY=replay)
    launch()
