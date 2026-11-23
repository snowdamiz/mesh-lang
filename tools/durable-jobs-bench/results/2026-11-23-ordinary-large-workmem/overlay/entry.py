#!/usr/bin/env python3
"""Apply the work_mem overlay at the existing disposable-cluster constructor."""

from hashlib import sha256
from pathlib import Path
import runpy
import sys

from workmem import instrument


def once(source, anchor, replacement):
    if source.count(anchor) != 1:
        raise ValueError(f"expected exactly one stock anchor: {anchor!r}")
    return source.replace(anchor, replacement)


def invoke(engine, mb, launcher, args, peer=None):
    if engine not in ("mesh", "oban", "go", "rust") or mb not in (4, 16):
        raise ValueError("expected a matrix engine and 4 or 16 MB work_mem")
    if args[-2] != "large":
        raise ValueError("expected the frozen large workload")
    out = Path(args[-1]).resolve()
    launcher = launcher.resolve(strict=True)
    sys.argv = [str(launcher), *args]
    sys.path.insert(0, str(launcher.parent))

    def configure(cluster):
        return instrument(cluster, mb, engine, out)

    if engine == "mesh":
        main = runpy.run_path(str(launcher))["main"]
        symbols = main.__globals__
        symbols["Cluster"] = configure(symbols["Cluster"])
        main()
        return

    source = peer.read_bytes()
    if sha256(source).hexdigest() != "8c2a090230503fbb1c9d0dcaa508ea3c8b42ea15fd2c9a84cddbc457840b0dbf":
        raise ValueError("the EXTERNAL peer overlay changed")
    source = source.decode()
    if engine == "oban":
        anchor = "    stock.update(Subject=Subject, Cluster=Cluster)"
        source = once(source, anchor, "    stock.update(Subject=Subject, Cluster=configure(Cluster))")
    else:
        anchor = '        exec(compile(source, path, "exec"), execute)'
        source = once(source, anchor, '''
        main_guard = 'if __name__ == "__main__":'
        source = once(source, main_guard, 'Cluster = configure(Cluster)\\n\\n' + main_guard)
        execute["configure"] = configure
        exec(compile(source, path, "exec"), execute)'''.lstrip("\n"))
    sys.argv = [str(peer), engine, str(launcher), *args]
    sys.path.insert(0, str(peer.parent))
    exec(compile(source, str(peer), "exec"),
         {"__name__": "__main__", "__file__": str(peer), "configure": configure, "once": once})


if __name__ == "__main__":
    engine, mb, launcher, *args = sys.argv[1:]
    peer = Path(args.pop(0)).resolve(strict=True) if engine != "mesh" else None
    invoke(engine, int(mb), Path(launcher), args, peer)
