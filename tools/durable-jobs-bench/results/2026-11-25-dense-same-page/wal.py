"""Reduce PG17 pg_waldump output for one heap's UPDATE records."""
import re

INSERT = re.compile(r"\bdesc: INSERT(?:\+INIT)? ")
UPDATE = re.compile(r"\bdesc: (?:HOT_)?UPDATE(?:\+INIT)? ")
FLAGS = re.compile(r"\bflags: 0x([0-9a-fA-F]{2})\b")
LENGTH = re.compile(r"len \(rec/tot\):\s*\d+/\s*(\d+)")
BLOCK = re.compile(r"\bblkref #(\d+): rel [\d/]+ blk (\d+)")


def reduce(text):
    totals = dict(inserts=0, insert_extent_pages=0, heap_extent_pages=0, updates=0,
                  same_page=0, hot=0, prefix=0, suffix=0, fpi=0, record_bytes=0)
    for line in text.splitlines():
        refs = BLOCK.findall(line)
        if refs:
            extent = max(int(block) for _, block in refs) + 1
            totals["heap_extent_pages"] = max(totals["heap_extent_pages"], extent)
            if INSERT.search(line):
                totals["inserts"] += 1
                totals["insert_extent_pages"] = max(totals["insert_extent_pages"], extent)
        if not UPDATE.search(line):
            continue
        blocks = [block_id for block_id, _ in refs]
        flags = int(FLAGS.search(line)[1], 16)
        fpi = "FPW" in line
        if blocks not in (["0"], ["0", "1"]):
            raise ValueError(f"unexpected jobs heap UPDATE block references {blocks}")
        if flags & 0x60 and (blocks != ["0"] or fpi):
            raise ValueError("WAL reuse flags require one heap block without a full-page image")
        totals["updates"] += 1
        totals["same_page"] += blocks == ["0"]
        totals["hot"] += "desc: HOT_UPDATE " in line
        totals["prefix"] += bool(flags & 0x20)
        totals["suffix"] += bool(flags & 0x40)
        totals["fpi"] += fpi
        totals["record_bytes"] += int(LENGTH.search(line)[1])
    return totals
