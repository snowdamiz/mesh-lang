# Dense producer broker context recovery

The splice repair passed the Linux nobody preflight. All 12 pilot arms then stopped before submission because the patched warm producer was wrapped in contextmanager twice. Inspecting the undecorated function still returned its decorator line, and the source patch recompiled that decorator. The second sibling removes the extra wrapping call. The initial SETUP, both failed pilots, and their pins remain intact.

The offline Linux nobody reproduction uses the exact staged source paths and the actual nested producer context. It reproduces the first retry's TypeError, then starts and stops a synthetic subprocess via the real context and session protocol under both repaired widths. The controller's in-grant preflight instead uses the retained native broker and PostgreSQL 17.11. It verifies a direct broker child, TLS and database durability settings, listens for the real readiness response, and closes the broker with zero submissions. Both preflights stop before workload execution. The original scorer and all workload gates are reused. The unmeasured macOS control also started and stopped the locally compiled broker over verified PostgreSQL 17.11 TLS at both widths using the frozen subject environment. Linux process identity checks run in the fenced native preflight. [Local compiled-broker receipt](evidence/local-macos.log) · [Local context proof](evidence/local-entry.log) · [First retry release and rejection](../timing-splice-retry/evidence/pilot/replay.json)

From the checkout root:

    d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/timing-context-verified
    python3 -B "$d/context_retry.py" check
    python3 -B "$d/local_entry.py"

The sole controller registers pilot.json byte for byte and opens a fresh build-free pilot after existing bookings. Its in-grant preflight and timed raw evidence go to evidence/pilot in this sibling. The same preflight runs under a separate peer grant after both pilot groups qualify and release.

    python3 -B "$d/context_retry.py" slot pilot "$(shasum -a 256 "$d/pilot.json" | cut -d' ' -f1)"
    python3 -B "$d/context_retry.py" replay pilot
    python3 -B "$d/context_retry.py" slot peers "$(shasum -a 256 "$d/peers.json" | cut -d' ' -f1)"
    python3 -B "$d/context_retry.py" replay peers

[Pilot request](pilot.json) · [Conditional peer request](peers.json)
