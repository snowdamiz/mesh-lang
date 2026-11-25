"""Run one readied or retained lane through the released stock SLOW composition."""

import os
from pathlib import Path
import sys

import stock_codec

ENV = "RIVER_BENCH_POOL_READINESS"


def main():
    engine, mode, launcher, *args = sys.argv[1:]
    if engine == "go":
        if mode not in ("stock", "cold", "warm", "underready") or ENV in os.environ:
            raise ValueError("invalid River readiness lane")
        if mode != "stock":
            stock_run = stock_codec.river_run

            def rebuilt(command, *, env, **kwargs):
                return stock_run(command, env={**env, ENV: mode}, **kwargs)

            stock_codec.river_run = rebuilt
    elif engine not in ("mesh", "oban") or mode != "peer":
        raise ValueError("invalid ordinary peer lane")
    stock_codec.run(engine, "pglz", Path(launcher), args)


if __name__ == "__main__":
    main()
