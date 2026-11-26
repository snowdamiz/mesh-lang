# SHORT16 progress at matched peer effect counts

Prewarmed River finished ahead of hot-checkout Mesh from submit in five of six released rotations. Even subtracting the entire Mesh producer time, with subsequent timings fixed, leaves Mesh behind in groups 1, 2, 3 and 5 by 31, 22, 61 and 36 ms. Mesh beat stock River from submit in four rotations and Oban in all six. [Original paired clocks](NATIVE-RESULTS.md)

The table aligns each engine on its own durable enqueue commit. At the elapsed time of prewarmed River’s 112th durable application effect, it counts how far Mesh had progressed. The grant column credits Mesh with every SQL grant whose millisecond stamp could already have occurred. A checked grant still needs a COMMIT reply before launch.

| Rotation | Mesh grants, upper bound | Mesh effects committed | Mesh jobs settled | River effects awaiting settlement |
|---|---:|---:|---:|---:|
| 1 | 85 | 69 | 69 | 112 |
| 2 | 72 | 66 | 57 | 96 |
| 3 | 57 | 49 | 43 | 112 |
| 4 | 75 | 60 | 59 | 112 |
| 5 | 73 | 65 | 59 | 95 |
| 6 | 100 | 96 | 92 | 112 |

With 16 seats charged through confirmed terminal receipts, reaching 112 effects requires at least 96 earlier durable settlements. At River’s deadline Mesh was 4–53 settlements short of that necessary condition in every rotation. Even immediate execution of every Mesh job already granted would leave 12–55 additional jobs requiring a grant. Those deficits overlap. They do not measure the time saved by changing either operation.

Mesh’s peak count of effect-committed jobs awaiting terminal commit was 10–14 across rotations. It used 113–121 terminal XIDs per 128 jobs. Warm River’s peak count was 111–128, with one or two terminal XIDs per rotation. On the same durable-enqueue clock Mesh completed all 128 application effects before Oban reached 16 in every rotation. The native comparisons retain each engine’s own settlement policy.

The separate SHORT fixed-8-ms writer used 16 and 18 terminal XIDs. Its refill-demand alternative missed its advancement gates. The remaining question is how many of those fixed-writer transactions can grant replacement jobs without an extra claim turn. A native performance result for that combination requires a separate matched comparison. [Fixed-writer and refill-demand pilot](../2026-11-25-short16-fixed8-refill/evidence/native/pilot-replay.json) · [Earlier fused SHORT comparison](../2026-11-24-short16-fused-refill/NATIVE-RESULTS.md)

Run `python3 -B peer_effect_progress.py` from this directory. The replay reads the 24 measured archives for Mesh, warm River, stock River and Oban. It checks the archive hashes, completed jobs, payloads, ordered application commits, all 256 application XIDs in each arm and both final clocks. The output includes both clock origins and ranks 16, 32, 64, 96, 112 and 128. [Replay output](evidence/peer-effect-progress.json)
