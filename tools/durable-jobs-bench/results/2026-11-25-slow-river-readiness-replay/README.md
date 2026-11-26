# SLOW River readiness replay correction

The original readiness SETUP released exit 1 after building the exact River ELF and before dispatching any arm. The retained-build sibling released exit 0 at 20:02:55Z with seven fresh raw archives. Its first offline replay failed when it compared integer listener PID keys with the decimal string keys written by JSON. The archives retain both outcomes and the toolchain supply receipt.

The replay re-runs the sealed scorers on all seven raw arms. For each of the four rebuilt lanes it requires exactly two positive integer PID keys in the recomputed `ready_detached_listeners` object, both expected River listener identities, and two canonical decimal strings in the recorded object. It converts those integer keys and compares every remaining key, type and value exactly. Duplicate JSON object keys fail before comparison. The qualified replay confirms 67 physical clients for warm River, seven for retained and rebuilt lazy River, and the expected pre-submit rejection at 66 physical clients for the under-ready arm.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-slow-river-readiness-replay
python3 -B "$d/replay.py" | cmp - "$d/qualified.json"
```

This command reads the archived releases, build and arm evidence. It has no dispatch action. The two historical proof trees are preserved byte for byte. [Qualification receipt](qualified.json)
