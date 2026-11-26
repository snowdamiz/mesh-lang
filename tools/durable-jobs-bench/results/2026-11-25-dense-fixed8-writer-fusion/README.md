# Dense fixed8 plus d388 writer fusion

The private writer-fusion SETUP, positive two-runner diagnostic and both reversed pilots released with exit 0 on November 25, 2026. The sealed binder verified the inherited correctness suite and eight fused fault cases on Linux ELF `ef6d785c741abd05560ba7cdbb2e90bcf7484f735f85224be30d818ad7811c2b`. The diagnostic recorded 960 replacement seats across 135 receipt-confirmed fused turns.

The pilot replay reconciles sixteen warmup and measured arms. Both measured fusion arms adopted 960 replacement seats, with four standalone grant-bearing turns per arm versus 147–152 across same-image off controls. Total grant-bearing turns fell to 141 and 140, and queue SQL calls fell to 693 and 688. The pilot gate stayed closed. Against the fastest off or bridge lane, group 1 saved 12.759 ms on submit through settlement and 22.146 ms through post-commit drain, both below threshold. Group 2 was 13.820 ms and 1.550 ms slower on those clocks and exceeded its runner and PostgreSQL CPU ceilings. Neither group passed the middle-write or terminal-window gates. The five peer blocks remain gated.

`evidence/native/setup` contains the raw proof, release, and binding. The diagnostic and pilot releases and replays are under `evidence/native/controller`.

## Replay the archived evidence

From the checkout root, run this command with Python 3.11 or later.

```sh
python3 -B tools/durable-jobs-bench/results/2026-11-25-dense-fixed8-writer-fusion/replay/reproduce.py
```

The launcher verifies the original [sealed packet](controller-packet.json), regenerates the setup binding, and scores the diagnostic and pilots from the 17 raw arms. It compares the pilot output with both the controller replay and the [independent replay](replay/independent-pilot-replay.json). It also checks the diagnostic output against its release replay, which predates the pilot archives. All commands run offline and read the archived evidence in place.

For a scoped Git export, include this result directory and each repository-relative path in [the dependency manifest](replay/dependencies.sha256). The result directory contains both exact source archives and the original driver.
