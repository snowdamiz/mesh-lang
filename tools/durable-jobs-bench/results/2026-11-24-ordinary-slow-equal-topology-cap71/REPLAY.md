# Verify the slow cap-71 packet offline

Run from the repository root. These commands check the sealed artifacts and archived native receipts. They do not start PostgreSQL or a workload.

```sh
packet=tools/durable-jobs-bench/results/2026-11-24-ordinary-slow-equal-topology-cap71
lane=target/ordinary-slow-equal-topology-cap71
(cd "$packet" && shasum -a 256 -c SHA256SUMS)
mkdir -p "$lane"
if [ ! -e "$lane/river-slow-2x32-cap71" ]; then
  tar -xzf "$packet/evidence/river-linux-amd64.tar.gz" -C "$lane"
fi
python3 -B "$packet/packet.py" check
python3 -B "$packet/offline.py"
```

Use `offline.py --sources` when the pinned local dependency and fixture source caches are available. The complete source replay produces `evidence/cap-proof.json`. To audit the untimed River ELF build, use `build-river.py` with the retained submit-timing source, a fresh output directory containing `tmp`, and the pinned Go module cache. The builder checks the source, stock ELF, and one-line change before recording the candidate digest.

The controller can replay new evidence without contacting the host.

```sh
python3 -B "$packet/packet.py" replay --mode setup --directory "$lane/native-setup"
python3 -B "$packet/packet.py" replay --mode timing --directory "$lane/native-timing"
```

The controller's execution commands are in the two intakes. They require matching request bytes in the controller inbox and an active assignment before any remote contact. Acquiring the checkout, active-arm, and native locks follows the stock recorder's sequence. A retry preserves each raw attempt and reruns only a block whose archive cannot qualify.
