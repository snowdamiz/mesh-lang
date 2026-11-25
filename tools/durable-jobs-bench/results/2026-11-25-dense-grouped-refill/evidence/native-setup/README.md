# Grouped refill SETUP

The controller released the pinned Linux build with exit 0 at 03:18:47 UTC.
The runner ELF is `d56d0e6112a9…`. The build used the independent stock
SQL and the exact sealed grouped refill source archive.

The released archive replays eight semantic cases. They cover SQL fencing,
held and lost terminal receipts, lost grouped receipt, cancellation, held
and forced teardown, and a partial cohort during stop.
[Release](controller-release.json) · [Proof binding](binding.json) · [Raw evidence](raw.tar.gz)
