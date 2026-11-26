#!/usr/bin/env python3
"""Paired per-group summary of evidence/{micro,live}.json (stdlib only)."""
import collections, json, statistics as st, sys
from pathlib import Path

E = Path(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).with_name("evidence"))
STOCK, OPT = ("stock", "stock-b"), ("optimized-a", "optimized-b")


def pct(new, old):
    return (new - old) / old * 100


def paired(rows, key, metric):
    groups = collections.defaultdict(dict)
    for r in rows:
        groups[(r[key], r["group"])][r["arm"]] = r[metric]
    out = {}
    for shape in sorted({k for k, _ in groups}):
        eff, ctl = [], []
        for (s, _), arms in groups.items():
            if s != shape:
                continue
            stock = st.mean(arms[a] for a in STOCK)
            eff.append(pct(st.mean(arms[a] for a in OPT), stock))
            ctl.append(pct(arms["stock-b"], arms["stock"]))
        out[shape] = dict(effect_median_pct=round(st.median(eff), 1), optimized_lower=f"{sum(e < 0 for e in eff)}/{len(eff)}",
                          control_median_pct=round(st.median(ctl), 1), control_range_pct=[round(min(ctl)), round(max(ctl))])
    return out


micro = [dict(r, ns_per_call=r["cpu_ns"] / r["iterations"]) for r in json.loads((E / "micro.json").read_text()) if r["mode"] == "cpu"]
alloc = {f'{r["shape"]}/{r["arm"]}': {k: r[k] for k in ("allocs", "reallocs", "moved_old_layout_bytes", "requested_bytes", "peak_live_requested", "frame_bytes", "explicit_bind_body_copy_bytes")}
         for r in json.loads((E / "micro.json").read_text()) if r["mode"] == "alloc" and r["arm"] in ("stock", "optimized-a")}
live = json.loads((E / "live.json").read_text())
print(json.dumps(dict(
    micro_cpu_ns_per_encode=paired(micro, "shape", "ns_per_call"),
    micro_alloc=alloc,
    live={m: paired(live, "shape", m) for m in ("cpu_seconds", "wall_ns", "peak_rss_bytes")},
    live_load_1m=[min(r["load_before"][0] for r in live), max(r["load_before"][0] for r in live)],
), indent=1))
