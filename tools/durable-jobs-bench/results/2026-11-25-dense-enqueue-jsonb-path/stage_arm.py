"""Add the producer DDL and untimed receipt counter to the released fixed8 arm."""

from pathlib import Path
import sys


def render(original, treatment):
    assert treatment in {"stock", "projected"}
    edits = {
        "import backend\n": "import backend\nimport overlay\n",
        "class Cluster(backend.Cluster):\n": """class Cluster(backend.Cluster):
    def reconcile_trial(self, *args, **kwargs):
        result = super().reconcile_trial(*args, **kwargs)
        counts = overlay.counts(self)
        assert counts == {"bulk": [8, 1024]}, counts
        backend.arm.h.write_json(self.evidence / "enqueue-cardinality.json", counts)
        return result

""",
        '            overlay = apply(cluster, "mesh", CANDIDATE)\n':
            f'            writer = overlay.apply_writer(cluster, {treatment == "projected"})\n'
            '            backend.arm.h.write_json(cluster.evidence / "enqueue-overlay.json", writer)\n'
            '            overlay_receipt = apply(cluster, "mesh", CANDIDATE)\n',
        'lane=LANE, observer=False, **overlay))': 'lane=LANE, observer=False, **overlay_receipt))',
    }
    for old, new in edits.items():
        assert original.count(old) == 1, old
        original = original.replace(old, new)
    compile(original, "factorial_arm.py", "exec")
    return original


if __name__ == "__main__":
    sys.stdout.write(render(Path(sys.argv[1]).read_text(), sys.argv[2]))
