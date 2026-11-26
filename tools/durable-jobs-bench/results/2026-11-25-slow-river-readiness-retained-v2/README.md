# Build-free SLOW River readiness proof

The original readiness SETUP released exit 1 before dispatching any arm. The stock rebuild matched the retained cap-71 SLOW ELF and the candidate build completed. Staging installed the exact code bundle and changed it to mode 0555. The shared audit helper then used the older codec proof's evidence directory and failed when exclusive creation encountered its existing log. Both earlier releases retain their evidence and disposition.

This sibling uses the same seven PGLZ diagnostics, order, inputs, scorers, stock engines and rebuilt River ELF. The stock ELF is `a23130f74210946291ed3609797e5b218ece5a7bd70df318851cf91e634da1e1`. The candidate ELF is `b591ed4603257d9bd48f83edb30a3947fe11708260679bbe53e8b3e8469cf6fb`. The readiness release and toolchain supply release are exact read-only inputs. The controller replays the retained build receipt, checks the native file pins, and audits the already staged code as the unprivileged benchmark user while holding both remote fences. The audit also checks the retained candidate's digest, ownership, permissions and executability before any arm. A missing or changed retained file stops the sibling at intake. [Retained custody](custody.json)

The controller writes the new grant, code-access log, file checks, inventories, and raw archives to this sibling's `evidence/proof`. The existing dispatcher uses this sibling's run ID for new remote worker directories. It leaves the existing native code and candidate directory read-only and reuses the sealed worker and scorers. The expected pre-submit under-ready rejection is the seventh diagnostic. Timing has no dispatch action here.

## Sole controller intake

Bind `request-proof.json` in the canonical checkout after the already released supply and failed readiness attempts. The original readiness exit of 1 is an explicit custody prerequisite. Its build and source-input receipts passed before staging stopped. From the canonical checkout root:

```sh
d=tools/durable-jobs-bench/results/2026-11-25-slow-river-readiness-retained-v2
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" proof
python3 -B "$d/controller.py" replay-proof
```

The successful sibling release must bind `request_sha256`, every entry in `raw-sha256.json`, and a `retained_custody_sha256` map containing the hashes of `retained-inputs.json`, `public-code-access.log`, and `retained-native-sha256.log` from the new proof directory. Replay checks all seven effective inventories and rejects missing or extra archives. Preserve partial captures if the sibling fails.
