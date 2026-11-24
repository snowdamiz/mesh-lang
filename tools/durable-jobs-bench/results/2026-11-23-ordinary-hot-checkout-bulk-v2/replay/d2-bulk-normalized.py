"""Run the sealed D2 bulk proof with exact device-ID normalization."""

from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
import sys
from tempfile import TemporaryDirectory


STATE = Path(__file__).resolve().parent
SOURCE = STATE.parent / "ordinary-hot-checkout/bulk-v2/native/dispatch.py"
spec = spec_from_file_location("sealed_d2_bulk", SOURCE)
sealed = module_from_spec(spec)
spec.loader.exec_module(sealed)
SETUP = "ordinary-hot-checkout-bulk-v2-normalized"
original_grant = sealed.grant
original_configure = sealed.configure


def grant(phase, key, seconds, companion_sha=None):
    if phase == "SETUP" and key == sealed.ID:
        key = SETUP
    return original_grant(phase, key, seconds, companion_sha)


def configure(path, attempt):
    boot, kernel, device, ticks, *files = path.read_text().splitlines()
    with TemporaryDirectory(dir=path.parent, prefix="identity-normalized-") as scratch:
        normalized = Path(scratch) / "native-identity.txt"
        normalized.write_text("\n".join([boot, kernel, device.strip(), ticks, *files]) + "\n")
        return original_configure(normalized, attempt)


if __name__ == "__main__":
    sealed.grant = grant
    sealed.configure = configure
    sys.exit(sealed.main())
