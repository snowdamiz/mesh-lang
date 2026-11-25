# FILE EXTERNAL grant parser failure

The controller released the original 24-minute TIMING grant with exit 1
at 02:28:27 UTC. The sealed timing command reached `matched.py:239`
before staging. The ledger expression returned five fields and
`grant()` raised `ValueError` while unpacking four. The grant JSON,
matrix staging, and timed arms were never created.

The [controller log](controller.log) and [release](controller-release.json)
record the attempted window. A local reproduction verified that a
four-field parser accepts that exact active TIMING row and rejects a
released, expired, or wrong-ID grant.
