# Public SLOW FILE and topology SETUP

The fenced Linode SETUP released with exit 1 at 04:14:27 UTC on November 25, 2026. All four cap-68 diagnostic archives are retained with their dispatch, stage, grant and release receipts. Each archive contains a `PermissionError` when `capture_runners` in `slow.py` calls `runner_backend_map` in `socket_map.py` under the reduced UID. The error occurs while enumerating the runner's `/proc/<pid>/fd`.

`failure-replay.json` records the raw hashes and the exact embedded adapter failures. The pilot and peers remain gated on a successful SETUP. The roster replacement is owned by the experiment implementer.
