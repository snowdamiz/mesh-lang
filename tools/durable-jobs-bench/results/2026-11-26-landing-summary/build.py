"""Collect the landing page's durable-jobs figures into comparison.csv.

Each workload reads the median submit-to-final-settlement time per engine
from its recorded series. Run from this directory: python3 build.py
"""

import csv
import json
from pathlib import Path

RESULTS = Path(__file__).resolve().parent.parent

# workload, jobs, rotations, source file, engine -> lane key, field
SERIES = [
    ('short', 128, 6, '2026-11-24-short16-river-readiness/evidence/summary.json', ('lanes',),
     {'Mesh': 'r2-d2', 'Oban': 'oban', 'River': 'river', 'Apalis': 'apalis'}, 'submit_to_settlement_ms'),
    ('slow', 512, 8, '2026-11-24-ordinary-slow-equal-topology-cap71/evidence/native-analysis/paired-scorecard.json', ('engines',),
     {'Mesh': 'mesh', 'Oban': 'oban', 'River': 'go', 'Apalis': 'rust'}, 'submit_ms'),
    ('large', 64, 5, '2026-11-25-public-large-file-external/grant-parser-retry/evidence/replay.json', ('clocks',),
     {'Mesh': 'file', 'Oban': 'oban', 'River': 'river', 'Apalis': 'apalis'}, 'submit_ms'),
]

# The dense peer series publishes its medians in PEER-RESULTS.md.
DENSE = {'Mesh': 1003.271, 'Oban': 1321.836, 'River': 1021.191, 'Apalis': 739.792}
DENSE_SOURCE = '2026-11-25-dense-fixed8-logical-factorial/PEER-RESULTS.md'


def main() -> None:
    rows = []
    for workload, jobs, rotations, source, path, lanes, field in SERIES:
        data = json.loads((RESULTS / source).read_text())
        for key in path:
            data = data[key]
        for engine, lane in lanes.items():
            rows.append([workload, jobs, rotations, engine, round(data[lane][field]['median'], 3), source])
    for engine, median in DENSE.items():
        rows.append(['dense', 1024, 5, engine, median, DENSE_SOURCE])
    with open(Path(__file__).with_name('comparison.csv'), 'w', newline='') as out:
        writer = csv.writer(out, lineterminator='\n')
        writer.writerow(['workload', 'jobs', 'rotations', 'engine', 'median_ms', 'source'])
        writer.writerows(rows)


if __name__ == '__main__':
    main()
