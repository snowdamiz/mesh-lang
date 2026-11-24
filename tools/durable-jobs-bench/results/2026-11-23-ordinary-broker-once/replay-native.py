"""Summarize the archived broker-only native series without contacting Linode."""

from pathlib import Path
import sys

import native


HERE = Path(__file__).resolve().parent
native.STATE = HERE / "evidence"
setup = native.STATE / "ordinary-broker-once-setup-14e25bc45d2d"
request = native.read(HERE / "native-request.json")
sys.exit(native.series("summarize", HERE / "native-freeze", request["freeze_sha256"], setup, []))
