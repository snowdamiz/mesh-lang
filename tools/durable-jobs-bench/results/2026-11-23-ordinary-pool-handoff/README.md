# Passing an application pool to ordinary Mesh actors

The trusted Jobs handler calls a service actor to obtain its PostgreSQL pool on every attempt. This component gave eight actors the same preopened pool as an argument and compared the two ways to obtain it. Each actor executed 64 attempt/effect pairs on two fixed scheduler workers. Every pair used two separate, ordered PostgreSQL commits over verified TLS. The identical release executable and live service ran in all three arms.

| Pool access | Median time for 512 pairs | Median process CPU per pair |
| --- | ---: | ---: |
| Service actor | 1,187 ms | 164.6 µs |
| Actor argument | 1,153 ms | 153.8 µs |
| Duplicate service control | 1,201 ms | 148.7 µs |

Passing the pool beat both controls on wall time in two of six rotations. Process CPU includes startup and shutdown, measured by the parent's consuming `wait4`. Host load averaged 43–46 on the shared 15-core Mac. The measurements reconcile 9,216 pairs and 18,432 independent writes. They give insufficient evidence to add a Jobs handler API for this optimization. [Compiled fixture source, results and verification driver](component.tar.gz)

## Replay

Run from the repository root with the existing release compiler, a matching runtime archive in `MESH_RT_LIB_PATH`, and PostgreSQL 17.11 in `PG_BIN_DIR`. The original measurements and their executable hash are under `evidence/pass-01` in the archive.

```sh
capture=tools/durable-jobs-bench/results/2026-11-23-ordinary-pool-handoff/component.tar.gz
echo "4cc24000c015b4da30614bd70341d61ebb50a50df7f01738bb19fd073e5a31c9  $capture" | shasum -a 256 -c -
out=$(mktemp -d "$PWD/target/actor-pool.XXXXXX")
tar -xzf "$capture" -C "$out"
cc -Wall -Wextra -Werror -O2 "$out/measure.c" -o "$out/measure"
target/debug/meshc build "$out/fixture" --opt-level 2 --output "$out/actor"
python3 -B "$out/run.py" replay
```
