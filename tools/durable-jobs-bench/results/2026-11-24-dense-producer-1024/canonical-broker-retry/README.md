# Canonical broker path recovery

The previous 12 arms reached runner capture, then rejected the broker path spelling with a parent-directory segment. The earlier preflight supplied a canonical path directly and missed that launcher path. The sibling now derives the broker from the canonical installed stage directory. The adapter requires that exact spelling, pins the ELF before opening the warm producer, launches with that same path, and carries the file stamp to capture. The existing identity checks verify the live child, argv, executable, affinity, and the original file stamp.

The network-disabled Linux control exercised the exact staged path as UID and GID 65534 using a local ELF. It rejected the old parent-directory spelling, a leaf symlink, a symlinked ancestor, a wrong digest, a different launch stamp, wrong argv, and replacement with identical bytes at a new inode. The original scorer still rejects all 12 released archives. [Local path evidence](evidence/local-identity.log) · [Previous release](../timing-context-verified/evidence/pilot/controller-release.json)

The retained Linux AMD64 broker and runner are required for the native smoke. The sole controller opens a five-minute correctness-only grant and runs one diagnostic arm for each width with the exact frozen input, original launcher chain, original recorder, and frozen scorer. The first rejected arm stops the smoke before the next launch. Each arm must reach runner capture and reconcile all 1,024 job receipts, one producer transaction, 2,048 independently committed application writes, and terminal cohorts. The smoke receipt exports correctness fields. Its diagnostic archives stay outside the benchmark series.

From the checkout root:

    d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024/canonical-broker-retry
    python3 -B "$d/controller.py" check
    bash "$d/local_identity.sh"

Only the sole controller registers the exact smoke request and opens its fresh grant after existing work:

    python3 -B "$d/controller.py" slot smoke "$(shasum -a 256 "$d/smoke.json" | cut -d' ' -f1)"
    python3 -B "$d/controller.py" replay smoke

Keep timing requests out of the inbox until the smoke release and full archived replay pass. Then the controller can admit the identical reversed pilot blocks. Peers remain contingent on that pilot.

    python3 -B "$d/controller.py" slot pilot "$(shasum -a 256 "$d/pilot.json" | cut -d' ' -f1)"
    python3 -B "$d/controller.py" replay pilot
    python3 -B "$d/controller.py" slot peers "$(shasum -a 256 "$d/peers.json" | cut -d' ' -f1)"

[Smoke request](smoke.json) · [Conditional pilot request](pilot.json) · [Conditional peers request](peers.json)
