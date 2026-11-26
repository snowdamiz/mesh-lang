# Dense enqueue launcher mode retry

The original Linode release stopped in the first stock warmup. Python refused to open the generated `factorial_arm.py` after the launcher switched to `nobody`. The worker sets `umask 077`, which gives the root-owned file mode `0600` when shell redirection creates it. The amendment explicitly sets mode `0644` on that file before the launcher calls `runuser`. [Original failed archive](evidence/original-warmup.tar.gz) · [Original release](evidence/original-release.json) · [Original sealed packet](../2026-11-25-dense-enqueue-jsonb-path/controller-packet.json)

The offline Linux proof used a pinned local Docker image with networking disabled. It executed the original and amended launchers, archived renderer and native wrapper, the real `runuser` transition, and the actual generated Python. A stub in the frozen harness path installed an audit hook that stopped Python just before the first product code would execute. The old launcher reproduced errno 13 under `077` and reached the hook under `022`. Both amended treatments reached the hook as UID 65534 under `077`, with the generated file at mode `0644` and the private output directory at mode `0700`. [Launch proof](evidence/local-launch.json)

The retry has its own seal, grant ID, staging directories and evidence root. It inherits the original two warmup and measured orders, frozen fixed8+d388 ELF and topology, SQL and scoring gates. All native access and dispatch remain with the sole controller.

From the canonical checkout root, run the offline seal check or replay.

```sh
d=tools/durable-jobs-bench/results/2026-11-25-dense-enqueue-jsonb-path-mode-retry
python3 -B "$d/controller.py" check
python3 -B "$d/controller.py" replay
```

The local launch proof can be repeated with `python3 -B "$d/prove_launch.py"` on a machine with the pinned image and a local Docker socket.
