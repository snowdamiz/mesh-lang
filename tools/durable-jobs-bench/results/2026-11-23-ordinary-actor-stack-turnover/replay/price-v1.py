"""Price actor turnover against the retained native dense clocks."""
import argparse
import json
from pathlib import Path

p = argparse.ArgumentParser(description=__doc__)
p.add_argument("timing", type=Path)
p.add_argument("native", type=Path)
a = p.parse_args()
timing = json.loads(a.timing.read_text())["timing"]
dense = json.loads(a.native.read_text())["rows"]["noop"]
assert len(dense) == 1
scores = dense[0]["scores"]
engines = ("ordinary", "oban", "river")
assert all(scores[name]["completed"] == 1024 and scores[name]["failed"] == 0 for name in engines)
rate_to_us = lambda rate: 1e6 / rate
clock = {}
for window in ("submit", "commit"):
    us = {name: rate_to_us(scores[name][window + "_jobs_per_s"]) for name in engines}
    clock[window] = dict(us_per_job=us, gaps_us={name: us["ordinary"] - us[name] for name in engines[1:]})

# Use the fast, fully reused, two-worker immediate cohort to project the CPU
# lever. The largest favorable paired CPU observation in any cohort is a
# deliberately generous sensitivity check, not a Linux performance bound.
main = timing["immediate/64/2"]["paired_stock_minus_reuse"]["full_cpu_us_per_actor"]["median"]
generous = max(row["paired_stock_minus_reuse"]["full_cpu_us_per_actor"]["max"]
               for row in timing.values())
cpu = scores["ordinary"]
baseline_runner_cpu = 1e3 * cpu["runner_cpu_ms_per_job"]
baseline_pinned_cpu = 1e3 * (cpu["pinned_user_cpu_ms_per_job"] + cpu["pinned_kernel_cpu_ms_per_job"])
cases = {}
for label, saved in (("paired_immediate_64", main), ("largest_favorable_cpu_pair", generous)):
    cases[label] = dict(us_per_attempt=saved, ms_per_1024=saved * 1024 / 1e3,
                        runner_cpu_pct=100 * saved / baseline_runner_cpu,
                        pinned_cpu_pct=100 * saved / baseline_pinned_cpu,
                        clocks={window: {"gap_closed_pct": {peer: 100 * saved / gap
                                                          for peer, gap in data["gaps_us"].items()},
                                         "optimistic_v1_jobs_per_s":
                                             1e6 / (data["us_per_job"]["ordinary"] - saved)}
                                for window, data in clock.items()})
print(json.dumps(dict(native_blocks=1, native_dense_clocks=clock, scenarios=cases,
                      native_v1_runner_cpu_us_per_job=baseline_runner_cpu,
                      native_v1_pinned_cpu_us_per_job=baseline_pinned_cpu), indent=2))
