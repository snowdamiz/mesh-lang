# Public LARGE FILE EXTERNAL grant parser retry

The original grant released exit 1 at 02:28:27 UTC before staging or starting any arm. Its parser raised a tuple unpacking error on the authorized TIMING line. The original request, controller traceback and exit-1 receipt are retained. The [journal excerpt](evidence/journal.txt) contains the actual grant, release and following grant.

This sibling reads the latest state record and accepts the exact retry ID and registered request during its TIMING interval. It binds the original five rotations, seven inventories, host and clocks by digest and calls the original timing and scoring functions. There is no native SETUP or build.

From the checkout root, run the offline checks.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-public-large-file-external/grant-parser-retry
python3 -B "$d/retry.py" check
python3 -B "$d/test_grant.py"
```

Only the sole Linode controller registers the new [packet](controller-packet.json) and dispatches its timing command in the existing recovery position. The original grant and request stay in the journal as released. Use the usual lock probes and archive the grant, raw arms and release under this sibling’s `evidence/native`. Run the packet’s replay command on the released evidence.
