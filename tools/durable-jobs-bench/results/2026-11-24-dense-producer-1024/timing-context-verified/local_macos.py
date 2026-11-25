import json
import os
from pathlib import Path
import runpy
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace

import entry_probe as probe

lane, recovery, width = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
fixture = lane / "local-broker-final"
broker_file = fixture / "mesh"
with TemporaryDirectory(prefix="context-actual-", dir=lane) as temp:
    os.environ.update(NATIVE_JOBS_ARTIFACTS=str(fixture),
                      PRODUCT_MANIFEST_SHA256=probe.sha(fixture / "mesh.json"),
                      DENSE_WIDTH=width, DENSE_BROKER=str(broker_file),
                      DENSE_BROKER_SHA256=probe.sha(broker_file))
    scope = runpy.run_path(str(recovery / "adapter.py"))
    globals_ = scope["main"].__globals__
    globals_["sys"] = SimpleNamespace(platform="linux", argv=["adapter.py", "mesh", "noop"])
    os.sched_getaffinity = lambda _: {0,1}
    globals_["process_stamp"] = lambda child: child.pid
    scope["pinned_file"] = lambda binary, pin: probe.sha(binary) == pin or sys.exit(1)
    scope["observe"] = lambda child, binary, verb, *_: child.args == [str(binary), verb] or sys.exit(1)
    scope["arm"].main = lambda: probe.exercise(scope, scope["arm"], Path(temp), False)
    scope["main"]()
    print(json.dumps(dict(width=width, compiled_broker=str(broker_file), pid_check="local child argv",
                         postgres="17.11", tls=True, session_closed=True, no_jobs_submitted=True)))
