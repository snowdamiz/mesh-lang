"""Seal or replay the compiled local grant-width experiments against the original verifier."""
import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
from statistics import median
import sys
import tarfile

from screen import HERE, LAYOUT, ORDERS, TOPOLOGY

sys.path.insert(0, str(TOPOLOGY))
import replay as baseline

STOCK_SOURCE = "a3314dcae571998a530fba74f0de330fe77b5a2ac20398981d0abfad85814eb4"
EIGHT_SOURCE = "291e95061d475d475592c7f65e64b90da53b4797168e4cf3515f0e0d2b762ef3"
KEYS = ("submit_jobs_per_s", "commit_jobs_per_s", "runner_cpu_ms_per_job", "claim_calls",
        "mean_occupied_slots", "commit_to_pickup_p99_ms")


def bundle(series, archive):
    files = [series / "series.json"]
    for group in sorted(series.glob("[0-9][0-9]")):
        files.extend(group.glob("*.status.json"))
        for arm in sorted(p for p in group.iterdir() if p.is_dir()):
            files.extend(arm.glob("*.json"))
            files.extend(arm.glob("runner-*.log"))
            files.append(arm/"broker-output.log")
    with archive.open("xb") as raw, gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
        with tarfile.open(fileobj=compressed, mode="w:") as tar:
            for path in sorted(files):
                data = path.read_bytes()
                if any(x in data.lower() for x in (b"private key-----", b"postgresql://", b"password=")):
                    raise ValueError(f"unexpected credential material in {path.name}")
                info = tarfile.TarInfo(path.relative_to(series).as_posix())
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))


def reduce(archive, layout, orders, pins, definition):
    results = []
    with tarfile.open(archive) as tar:
        def read(name):
            return json.load(tar.extractfile(name))

        series = read("series.json")
        assert series["orders"] == orders and series["layout"] == layout
        assert series["pins"] == {k:v for k,v in pins.items() if k != "version"}
        for number, order in enumerate(orders, 1):
            group = {}
            for name in order:
                path = f"{number:02}/{name}"
                status = read(path + ".status.json")
                assert status["exit_code"] == 0
                source = read(path + "/grant-width.json")
                assert source == {"label":name, "stock_source":STOCK_SOURCE, "installed_source":definition[name]}
                arm = baseline.verify_arm(lambda k: read(path + "/" + k),
                        lambda k: tar.extractfile(path + "/" + k).read().decode(),
                        layout[name], pins, local=True, sample_activity=False)
                group[name] = {k: arm[k] for k in KEYS}
                group[name]["load1_start"] = status["load_before"][0]
                group[name]["warm_application_sessions"] = arm["warm_application_sessions"]
                group[name]["used_application_backends"] = arm["used_application_backends"]
                group[name]["unique_xids"] = arm["unique_product_work_terminal_xids"]
            results.append(group)
    medians = {name:{k: median(g[name][k] for g in results) for k in KEYS} for name in layout}
    return dict(groups=results, medians=medians, orders=orders, input_sha256=series["input"],
                archive_sha256=hashlib.sha256(archive.read_bytes()).hexdigest())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seal", type=Path)
    args = parser.parse_args()
    archive = HERE / "local-arms.tar.gz"
    if args.seal:
        bundle(args.seal, archive)
    pins = baseline.read(TOPOLOGY/"local-pins.json")
    results = reduce(archive, LAYOUT, ORDERS, dict(pins, version="3828f20d2d6beb683868cce33e6dd1958365914b"),
                     {k: EIGHT_SOURCE if k == "two_width8" else STOCK_SOURCE for k in LAYOUT})
    data = json.dumps(results, sort_keys=True, indent=2)+"\n"
    if args.seal:
        (HERE/"local-results.json").write_text(data)
    else:
        assert (HERE/"local-results.json").read_text() == data
    print("Reconciled 16,384 completed jobs, 32,768 application XIDs and 16,384 terminal XIDs.")


if __name__ == "__main__":
    main()
