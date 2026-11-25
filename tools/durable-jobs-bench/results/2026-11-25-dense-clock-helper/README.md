# Per-job clock calls in the bulk producer

Keep the released producer. A 30 ms improvement across eight batches of 128 jobs would require 29.3 µs of elapsed saving per job if every saving reached the submit clock. Five native 128-job SHORT observer arms recorded 0.190–0.328 ms per batch inside the clock expression, or 1.49–2.57 µs per call. Scaling those readings to eight batches gives 1.52–2.63 ms. The candidate still executes that expression for each input row. Its potential saving is the surrounding SQL-function call, which the nested timer does not isolate. Both observer modes failed their original perturbation gates. These are instrumented SHORT observations, and the dense saving remains unmeasured. [Native observer and limits](../2026-11-24-r2-d2-producer-boundary/evidence/recovery/README.md)

On PostgreSQL 17.11, replacing the stock clock helper with a SQL-standard expression allowed inlining. The bulk layout check then rejected its changed definition, and the same 128 valid inputs returned the stock fallback marker. A narrower proof restored the original helper and placed its volatile expression directly in the guarded writer. Each writer returned 128 ordered receipts and stored matching job IDs, request keys, fingerprints and schedule timestamps. The narrow version reduced clock-helper calls from 128 to zero while preserving eligibility for the bulk path. This witness covers the database writer. The exact dense producer’s eight stock executor calls account for about 104–133 ms across the whole workload, and include many other operations. [Dense native measurements](../../../../../t3-d47befee/tools/durable-jobs-bench/results/2026-11-25-dense-producer-layout/README.md)

Run from the checkout root with a local PostgreSQL 17.11 binary directory. The probe starts a socket-only disposable database under `target/`, uses the SQL from commit `b3557273d`, and checks the five original raw hashes. It runs without workstation performance timing.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-clock-helper
python3 -B "$d/proof.py" /opt/homebrew/opt/postgresql@17/bin | cmp - "$d/proof.json"
```
