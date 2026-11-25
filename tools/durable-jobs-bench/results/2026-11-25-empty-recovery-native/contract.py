"""Frozen identities and the only allowed procedure change."""
from hashlib import sha256
from pathlib import Path
import re

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
REVISION = "b3557273d72be3802bf189b284e7caaf5f3caa0f"
ELF = "af2aca867a02d526cb6ba92d1867166ffc346f31e97a8f8aba0f19aa76b1d355"
MANIFEST = "979cd601099c5e7da4be763229a91e9dcabca51a35f4bed79f6cf4128adbac55"
INPUT = "9afb598e8e80b56fc96689cfbe7a7061d82f8e5e3c33a2c534d680ce3db2528a"
BOOT = "90ec1c24-08c8-4bc1-aa28-79b6db1bd13a"
STOCK = "c5e348c3f324c1e0a7b7d38715be47ae557b1bcfd46eb63e4edeffc9c5046763"
CANDIDATE = "86a17110290c835a983cbdae9feb0a5007d397d8aab3300b2d018fda52cecbc9"
ANCHOR = "FROM due WHERE j.namespace = ns AND j.id = due.id;"
CHANGE = ANCHOR[:-1] + "\n        AND EXISTS (SELECT 1 FROM due);"
BASE = "/var/lib/durable-jobs-bench/native"
POSTGRES = "b70863988c4762b6b914ac54d194fa5a1723201100d8155caf7781edba29faa1"
DEPTHS = (0, 1024, 16384)
PHASES = ("gate-stock", "stock", "candidate")


def digest(data):
    return sha256(data).hexdigest()


def definitions(candidate):
    assert digest(candidate) == CANDIDATE
    candidate = candidate.decode()
    assert candidate.count(CHANGE) == 1
    stock = candidate.replace(CHANGE, ANCHOR)
    assert digest(stock.encode()) == STOCK
    return dict(stock=stock, candidate=candidate)


def body(sql):
    return sql.split(" AS $$", 1)[1].rsplit("$$;", 1)[0]


def update(sql):
    query = body(sql).split("    WITH due AS (", 1)[1].split(";", 1)[0]
    query = "WITH due AS (" + query
    return re.sub(r"\b(ns|page_size|cutoff|queue_filter)\b",
                  lambda m: {"ns": "$1", "page_size": "$2", "cutoff": "$3",
                             "queue_filter": "$4"}[m[0]], query)


def rewrite(source, root, phase):
    assert phase in PHASES
    for before, after, count in (
        ('source="$(cd "$here/../../../../.." && pwd -P)"', f'source="{root}"', 1),
        ('"$here/product_arm.py" "$@"', f'"$here/observe.py" {phase} "$@"', 2),
    ):
        assert source.count(before) == count, before
        source = source.replace(before, after)
    return source
