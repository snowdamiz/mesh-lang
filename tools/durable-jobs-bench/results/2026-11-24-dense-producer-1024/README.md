# Private dense Mesh producer width

The same compiled broker submits the frozen 1,024 × 1 KiB dense input through eight 128-spec writer calls or one private 1,024-spec writer call. Each arm uses one outer producer transaction. Migration and both 32-slot runners use the exact measured logger-off grouped-hot b3557273 ELF, SHA-256 af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355. The two identical 128 controls and wide treatment differ by one broker environment flag. Two independent application commits per job and the same grouped terminal transaction contract are required in every Mesh arm.

The six earlier grouped-hot measurements spent 154.5–179.2 ms from submission to recorded commit, 73.2–100.4 ms behind their paired River arms. All six used eight set writes. The earlier SQL component also used 128-spec groups and prevalidated inputs. The short producer boundary, large-file broker and runner-readiness experiments cover different paths. Width can reduce producer work. The paired drain clock measures any accompanying shift in completion. [Grouped raw-ledger replay](../2026-11-24-dense-product-d2-transfer/NATIVE-RESULTS.md) · [Prior SQL component](../2026-11-23-set-based-enqueue-component/README.md)

## Proof and bounds

The private runtime is based on 1e70f18d with its existing checkout code from 916cfe53. The changed files live on t3/dense-producer-1024-private. The experiment uses the normal compiled Mesh API, the original spec preparation, request-key fingerprint, savepoint and namespace fence, and the PostgreSQL 17 layout and migration checks. A separately granted private SQL function reuses the v1 writer and accepts only 1,024 rows. Public calls retain the 128-spec and 1 MiB limits. The private raw frame is capped at 2 MiB, the normalized bind at 1,800,000 bytes, and each projected receipt at 512 bytes. SQL returns at most 1,025 bounded rows to distinguish an oversized reply. The caller verifies all 1,024 ordinals and receipts before releasing the savepoint.

The compiled local proof ran against disposable PostgreSQL 17.11 over verified TLS. It observed 8 × 128 and 1 × 1,024 SQL result rows, reconciled every durable receipt and payload in input order, and checked each enqueue record against its outer commit. It also checked matching and conflicting keys across positions 128 and 129 using both paths, uniqueness fallback, outer and savepoint rollback, layout, ACL and migration fallback, the namespace fence, public and private limits, oversized binds and short, oversized, extra and malformed replies. The loopback relay dropped acknowledgements both before commit and after durable commit. An explicit retry restored the full receipt set with no duplicate durable work. The local proof stores verdicts and durable rows, with no workload timing. The native SETUP repeats the proof with the exact staged Linux broker and original migrator before either pilot.

## Controller sequence

The sole fenced Linode controller owns all native setup, timing, archive export and release. The SETUP and pilot requests go behind existing reservations. Both warmups and measured arms must reconcile in each of two reversed strict pilots. The candidate must save more than the largest of 10 ms, 5% of the faster 128 control's submit-to-commit interval, and the duplicate spread in each measured pair. Its drain may exceed the faster control by at most the largest of 10 ms, 2% of that control's drain, and observed duplicate drain spread. Report submit-to-terminal and all per-arm clocks directly from each ledger.

After both pilots pass and release, two new reversed matched groups include both 128 controls, the 1,024 candidate, and the pinned stock River, Oban and Apalis equal-topology lanes. All attempts use separate evidence paths. Retain raw archives, failures, and release records. A failed arm cannot contribute a clock.

From the repository root:

    d=tools/durable-jobs-bench/results/2026-11-24-dense-producer-1024
    python3 -B "$d/controller.py" check

    # Sole controller under the matching grants
    python3 -B "$d/controller.py" setup
    python3 -B "$d/controller.py" timing pilot
    python3 -B "$d/controller.py" replay pilot
    python3 -B "$d/controller.py" timing peers
    python3 -B "$d/controller.py" replay peers

[Local verdicts](evidence/local-proof.json) · [Raw local proof](evidence/local-proof-raw.tar.gz) · [Retained local failures](evidence/local-failures.tar.gz) · [Frozen request](request.json)
