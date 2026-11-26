# Public LARGE canonical string qualification and component screen

This packet extends the source-only experiment in `c9d9ef052` on the exact public `b3557273` source. The original candidate and historical releases retain their byte identities. The sibling candidate uses the ceiling of half the complete frame length and selects a field after existing validations and policy normalization. The comparison also transfers the prior args-and-metadata move and serde serializer unchanged onto the same public source.

The custom encoder copies one decoded Rust `&str` only when it is an immediate args value at least 64 KiB long and at least half the complete frame. The scan excludes U+0000 through U+001F, quote, and backslash. Keys and sibling fields stay with serde, including nested containers, other large strings, numeric formatting and map order. Inputs use the existing untagged decoder and caps. The probe rejects malformed byte sequences through the public runtime's File.read entry point. The public C ABI assumes a valid Mesh String and cannot safely be fed a forged invalid UTF-8 value. Embedded NUL in decoded JSON stays escaped and exercises the existing PostgreSQL error path. The audit found no changes to either SQL writer, COMMIT, or the test and validation callers.

## Controller handoff

The [packet](controller-packet.json) records the exact remote argument vectors, capsule digest, local input digests and releases required for the next phase. [Original archive and prior releases](dependencies.json) are read-only inputs. From the repository root set:

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-canonical-string-linux
python3 -B "$d/pack.py" | shasum -a 256
```

Register the exact packet bytes for each active phase ID through the sole authenticated capacity driver and use a fresh 12-hex suffix. Keep the existing queue order and both native locks. Only SETUP accepts the regenerated capsule on stdin. Use exact UTF-8 bytes from `native.sh` for the `bash -c` argument. The native command emits the phase summary hash. Export under the same fenced controller with the phase's hash and retain stdout as `raw.tar.gz`. On failure preserve the native `verify` or component output directory and controller logs before considering a new suffix. The native command traps its own disposable source and Cargo trees. Its sealed input, executable and proof directories are retained for the earned screen.

1. Qualification SETUP compiles stock, prior move and corrected candidate as separate release images from the public source archive. The probe moves the identical production preparation statements into one private helper shared by the production entry and component. It compares exact byte streams and exact errors against stock, checks both FILE profiles, verifies fast-path eligibility and runs the prior corpus with new targeted cases. The compiled public Jobs ABI exercises scalar and set-based writes, outer rollback and savepoint recovery, request-key retries in every writer direction, ID/receipt/fingerprint agreement, SQL errors, deferred COMMIT failure, and connection cuts before COMMIT and after its durable result. A loopback PostgreSQL cluster belongs to this phase and is torn down when it exits.
2. Component TIMING requires a successful SETUP release and matching offline qualification replay. It uses the original CPU 0 affinity, eight submissions per process, six stock-A/candidate/stock-B orderings and 1% steal limit. Move-only is placed in a rotating slot and the late-escape series receives all six orders as a diagnostic. Each row must consume 64 frames and generate the frozen plain digest or the qualified fallback digest. The plain candidate earns the next stage only at median CPU savings of at least 40 ms per full submission versus the faster stock duplicate, with savings strictly above duplicate CPU spread and positive wall savings in at least five rotations. This gate uses fresh public-source clocks.
3. The separately parked FILE comparison, if earned, keeps the original broker image, duplicate FILE controls, input, EXTERNAL storage, frozen role topology and both clocks. It needs a separate build/qualification and then a separate timing grant.

Use Python 3.12+ to replay the returned archives. `qualification-dir` and `screen-dir` each contain the controller's `controller-release.json` and raw export. Bind `request_sha256` to the exact packet bytes and `raw_sha256` to the controller export. Both receipts carry the same `suffix`. The screen release carries `qualification_sha256` from replay.

```sh
python3 -B "$d/replay.py" qualification qualification-dir
python3 -B "$d/replay.py" screen qualification-dir screen-dir
```

No build, test, database or workload ran during packaging. The runtime change, platform qualification and cost gate remain unmeasured.
