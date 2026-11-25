# Dense logical grant key on native PostgreSQL

The exact owner-verified candidate carries namespace into the rendered row, retains scalar namespace and ID equalities, and adds a closed full-key BETWEEN with identical endpoints. The pinned PostgreSQL 17.11 proof covers the cold prepared-plan case, all 15 analyzed plans, both EPQ races and the unchanged stock contract. The namespace-only predecessor produced a different broad scan in the cold case. [Candidate](candidate.sql) · [Proof](proof.json)

The packet uses the public b3557273 grouped-hot 2×32 lane, guarded bulk enqueue, the original ELF, broker and migration, 32 warmed application TLS connections and the existing two-CPU cap. It installs the replacement function after migration in each fresh trial database and verifies function ownership, ACL, invoker execution and configuration. No runtime changes are needed.

Group 0 runs stock and treatment as separate compiled arms with a committed fresh 1,024-job backlog after real pre-submit idle priming. Each verifies the worker contract with real TLS roles in a disposable clone. Sampled live grant plans are attributed to the two runner backend rosters. The treatment must show point-key or equivalently bounded UPDATE access in positive drain plans and no target visits in empty plans. The retained successful stock capture is a reference for confirming the broad plan if the fresh stock happens to choose a bounded plan. Group 0 clocks are excluded from all speed comparisons.

The two opposite-order A/key/B and B/key/A rotations keep the stock observer and use no plan instrumentation. Submit-to-terminal and enqueue-commit-to-terminal must each beat the faster duplicate by more than the duplicate spread and 5% in both groups. Each arm proves 1,024 terminal jobs, 2,048 distinct application write XIDs, a separate durable enqueue transaction, the grouped completion contract and the same limits. The resource gate uses the existing host and runner CPU measures plus passive procfs PostgreSQL bracketing outside the clock. Its readback reports paired, unmatched and bracket slack. Comparison of PostgreSQL process CPU is scoped to stable PIDs.

Matched River, Oban and Apalis have a separately gated recipe with the same public 2×32 inventories and five full warmup/measured blocks. It requires a clean primary release and a passing primary gate. No peer booking is made by primary intake. The fresh-snapshot fallback remains held. Original releases, failed attempts and raw archives stay in place.

Run these from the checkout root. The sole controller registers the exact packet and runs intake behind existing bookings.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-logical-grant-key-native
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" intake "$(shasum -a 256 "$d/controller-packet.json" | cut -d ' ' -f 1)"
python3 -B "$d/controller.py" replay
```
