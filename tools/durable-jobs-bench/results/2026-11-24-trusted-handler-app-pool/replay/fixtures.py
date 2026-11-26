"""Derive the timing fixtures from the exact archived V1 main module."""
import argparse
import hashlib
from pathlib import Path
import shutil

FROZEN_MAIN = "06ac61eb204d1ebe0d45fff1d6694ff76536210b6a56613ac43b9aea94598efd"
WRITES = "1b5ab2f2459ce2e33d04d7d4c96d9c9e3cf0ac7cf7de90eeb1ec2ca838e9bee0"
FILES = ("main.mpl", "writes.mpl", "worker_identity.mpl", "worker_identity.c")
MODES = ("service", "direct", "service-observed", "direct-observed")


def replace(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f"expected one frozen fixture anchor {old!r}")
    return text.replace(old, new)


def main_source(source, mode):
    direct = mode.startswith("direct")
    observed = mode.endswith("-observed")
    if direct:
        service_begin = source.index("service ApplicationPool do")
        service_end = source.index("fn perform(", service_begin)
        source = source[:service_begin] + source[service_end:]
        source = replace(source, "fn perform(context :: String, entered_us :: Int)",
                         "fn perform(context :: String, entered_us :: Int, app_pool :: PoolHandle)")
        source = replace(source, '  let pool = ApplicationPool.current(Process.whereis("parity-app-pool"))\n',
                         "  let pool = app_pool\n")
        source = replace(source, "fn handle(context :: String)", "fn handle(context :: String, app_pool :: PoolHandle)")
        source = replace(source, "case perform(context, entered_us)", "case perform(context, entered_us, app_pool)")
        source = replace(source, "fn serve_started(pool :: PoolHandle)", "fn serve_started(pool :: PoolHandle, app_pool :: PoolHandle)")
        source = replace(source, "Jobs.start(pool, trusted, handle)", "Jobs.start_with_app_pool(pool, trusted, handle, app_pool)")
        source = replace(source, '''  let owner = ApplicationPool.start(app_pool)
  let registered = Process.register("parity-app-pool", owner)
  let result = if registered == 0 do serve_started(pool) else Err("cannot register application pool") end''',
                         "  let result = serve_started(pool, app_pool)")
    if observed:
        lookup = ("  let pool = app_pool\n" if direct else
                  '  let pool = ApplicationPool.current(Process.whereis("parity-app-pool"))\n')
        source = replace(source, lookup, '  let lookup_begin_us = realtime_us()\n' + lookup +
                         '  let lookup_end_us = realtime_us()\n')
        source = replace(source, '  if hold > 0 do Timer.sleep(hold) end\n',
                         '  let first_return_us = realtime_us()\n  if hold > 0 do Timer.sleep(hold) end\n')
        source = replace(source, '  let _ = record_effect(pool, ns, key)?\n',
                         '''  let _ = record_effect(pool, ns, key)?
  let second_return_us = realtime_us()
  IO.eprintln("APP_POOL_JOB " <> Json.encode(json {
    job_id: job_id, job_key: key, attempt: attempt, entered_us: entered_us,
    lookup_begin_us: lookup_begin_us, lookup_end_us: lookup_end_us,
    first_return_us: first_return_us, second_return_us: second_return_us
  }))
''')
        if not direct:
            source = replace(source, '  let result = if registered == 0 do serve_started(pool) else Err("cannot register application pool") end\n',
                             '''  let result = if registered == 0 do
    let resolved = Process.whereis("parity-app-pool")
    if resolved == owner do
      let _ = ApplicationPool.current(resolved)
      IO.eprintln("APP_POOL_REGISTRATION_OK")
      serve_started(pool)
    else
      Err("registered application service did not return the application pool")
    end
  else
    Err("cannot register application pool")
  end
''')
    return source


def generate(source, output):
    original = (source / "main.mpl").read_bytes()
    assert hashlib.sha256(original).hexdigest() == FROZEN_MAIN
    assert hashlib.sha256((source / "writes.mpl").read_bytes()).hexdigest() == WRITES
    output.mkdir()
    for mode in MODES:
        folder = output / mode
        folder.mkdir()
        for name in FILES[1:]:
            shutil.copyfile(source / name, folder / name)
        (folder / "main.mpl").write_text(main_source(original.decode(), mode))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("frozen", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    generate(args.frozen, args.output)
