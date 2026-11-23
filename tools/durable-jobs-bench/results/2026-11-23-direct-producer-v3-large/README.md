# Direct-file Mesh producer under stock storage policy

The first large rotation on November 23, 2026 ran original PostgreSQL
storage policy on the two-CPU native host. It used duplicate stock Mesh
arms, a direct-file Mesh producer with the same compiled worker, and
fresh Oban, River, and Apalis arms. The raw ledger matched all 384 measured
jobs and 768 independently committed application writes.
[Exact inputs and executables](series.json), [one-rotation evidence](summary-partial.json)

The direct producer took 1,540 ms to reach durable enqueue commit.
The duplicate stock arms took 1,839 and 1,842 ms. Submit-to-settlement
throughput was 26.84 jobs/s for direct-file Mesh and 23.68–24.03 for its
stock controls. Commit-to-settlement throughput was 75.84 jobs/s, within
the two stock results of 74.12 and 77.92. The five-group EXTERNAL and
symmetric treatment series use separate inventories.
