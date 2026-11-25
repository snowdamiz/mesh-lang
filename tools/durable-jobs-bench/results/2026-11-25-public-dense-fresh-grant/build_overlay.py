"""Build the trial function from its pinned public SQL declaration."""

from hashlib import sha256
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
STOCK_SHA256 = "31597202cdd0c8d03c8472e51362f0cfc6a988ca28a8e6e29fb1c69825c8ec1a"
SIGNATURE = "mesh_jobs.claim_batch_set_v1(text,text,jsonb,bigint,integer,bigint)"


def definition(schema):
    pattern = r"CREATE OR REPLACE FUNCTION mesh_jobs\.claim_batch_set_v1\(.*?^\$\$;"
    return re.search(pattern, schema, re.S | re.M)[0] + "\n"


def body(ddl):
    return re.search(r"\bAS (\$\w*\$)(.*?)\1", ddl, re.S)[2]


def overlay(stock):
    if sha256(stock.encode()).hexdigest() != STOCK_SHA256:
        raise ValueError("public stock function does not match the pinned source")
    start = stock.index("    RETURN QUERY\n    WITH picked")
    stop = stock.index("    IF FOUND THEN RETURN; END IF;", start)
    stop += len("    IF FOUND THEN RETURN; END IF;\n")
    selection = stock[start:stock.index("    ), granted AS", start)]
    selection = selection.removeprefix("    RETURN QUERY\n")
    selection = selection.replace("SELECT j.* FROM", "SELECT j.*, j.tableoid AS rel, j.xmin AS version_xid, j.ctid AS loc FROM")
    selection = selection.replace("            gen_random_uuid() AS grant_token,",
                                  "            p.rel, p.version_xid, p.loc, p.attempt + 1 AS next_attempt,\n"
                                  "            gen_random_uuid() AS grant_token,")
    routines = (HERE / "fresh.sql").read_text()
    declarations, layout, manifest_end, update, render = (
        section + "\n" for section in routines.rstrip().split("\n-- section\n")
    )
    fast = layout + "".join(
        "    " + line if line.strip() else line
        for line in (selection + manifest_end + update + render).splitlines(True)
    ) + "    END IF;\n"
    legacy = "".join("    " + line if line.strip() else line for line in stock[start:stop].splitlines(True))
    result = (stock[:start] + fast + "    IF same_relation IS DISTINCT FROM true THEN\n"
              + legacy + "    END IF;\n" + stock[stop:])
    return result.replace("    due_at bigint;\n", "    due_at bigint;\n" + declarations, 1)


if __name__ == "__main__":
    print(overlay(definition(Path(sys.argv[1]).read_text())), end="")
