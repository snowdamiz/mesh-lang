"""Attach the hook to the same frozen direct-observed app-pool harness."""
import importlib.util
import json
from pathlib import Path
import shutil
import sys
import time

HERE = Path(__file__).resolve().parent

# The worker permits traversal of the stage and denies directory listing.
boundary_spec = importlib.util.spec_from_file_location("boundary", HERE / "boundary.py")
boundary = importlib.util.module_from_spec(boundary_spec)
boundary_spec.loader.exec_module(boundary)
clock_bracket, require = boundary.clock_bracket, boundary.require


def attach(arm, module, enabled):
    base = arm.Cluster

    class Cluster(base):
        boundary = None

        def anchor(self, engine):
            clock = clock_bracket()
            before_ns = time.time_ns()
            row = json.loads(self.sql(engine, """
              SELECT json_build_object('xid', pg_current_xact_id()::text,
                'sql_us', (extract(epoch FROM clock_timestamp()) * 1000000)::bigint,
                'system_identifier', (SELECT system_identifier::text FROM pg_control_system()))
            """))
            return dict(row, clock=clock, before_ns=before_ns, after_ns=time.time_ns())

        def prepare_database(self, engine):
            super().prepare_database(engine)
            require(engine == "mesh", "expected private Mesh benchmark database")
            (self.data / "mesh-postflush").mkdir(mode=0o700)
            self.sql(engine, f"""
                ALTER ROLE compiled_caller IN DATABASE mesh SET session_preload_libraries TO '{module}';
                ALTER ROLE compiled_caller IN DATABASE mesh SET mesh_postflush.enabled TO '{"on" if enabled else "off"}';
            """)
            self.boundary = json.loads(self.sql(engine, """
              SELECT json_build_object(
                'parallel_workers', current_setting('max_parallel_workers')::int,
                'max_prepared_transactions', current_setting('max_prepared_transactions')::int,
                'synchronous_standby_names', current_setting('synchronous_standby_names'),
                'wal_sync_method', current_setting('wal_sync_method'))
            """))
            self.boundary.update(enabled=enabled, start=self.anchor(engine))

        def reconcile_trial(self, engine, *args):
            result = super().reconcile_trial(engine, *args)
            self.boundary["end"] = self.anchor(engine)
            return result

        def stop(self):
            super().stop()
            if self.boundary is None:
                return
            for path in sorted((self.data / "mesh-postflush").glob("*.log")):
                require(path.is_file() and not path.is_symlink() and path.stem.isdecimal(), "invalid backend export path")
                shutil.copyfile(path, self.evidence / f"postflush-{path.name}")
            self.boundary["exports_collected_ns"] = time.time_ns()
            arm.h.write_json(self.evidence / "postflush-arm.json", self.boundary)

    arm.Cluster = Cluster


def load_app(module, mode):
    require(mode in {"on", "off"}, "unexpected module mode")
    module = Path(module)
    require(module.is_absolute() and module.is_file() and not module.is_symlink()
            and all(c.isascii() and (c.isalnum() or c in "/._-") for c in str(module)), "invalid module path")
    spec = importlib.util.spec_from_file_location("app_pool", HERE / "native-arm.py")
    app = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(app)
    load = app.load_arm

    def load_arm(path):
        arm = load(path)
        attach(arm, module, mode == "on")
        return arm

    app.load_arm = load_arm
    return app


def main():
    source, runner, out, module, mode = sys.argv[1:]
    app = load_app(module, mode)
    app.main(Path(source), Path(runner), "direct-observed", "noop", Path(out))


if __name__ == "__main__":
    main()
