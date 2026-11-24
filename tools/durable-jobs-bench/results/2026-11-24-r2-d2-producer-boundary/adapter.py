"""Generate a narrow overlay of the sealed short R2 adapter for observed lanes."""
from pathlib import Path
import sys

from materialize import replace


def split_source(stock):
    return replace(stock, "    arm.Subject = SplitSubject\n    arm.main()",
                   "    from observe import install\n    install(arm, SplitSubject)\n    arm.main()")


def launch_source(stock):
    return replace(stock, '    "EXIT_BULK_CHECKOUT=$mode")',
                   '    "EXIT_BULK_CHECKOUT=$mode"\n'
                   '    "R2_OBSERVATION=${R2_OBSERVATION:?}"\n'
                   '    "R2_OBSERVED_BROKER=${R2_OBSERVED_BROKER:?}"\n'
                   '    "R2_OBSERVED_BROKER_SHA=${R2_OBSERVED_BROKER_SHA:?}"\n'
                   '    "R2_OBSERVED_MANIFEST_SHA=${R2_OBSERVED_MANIFEST_SHA:?}")')


def trial_source(stock):
    stock = replace(stock, '[*self.command, "session"], env=env,',
                    '[str(self.probe_broker), "session"], env=dict(env, MESH_R2_PRODUCER_PROBE="1"),')
    stock = replace(stock, '                broker.stdin.write(json.dumps(command, separators=(",", ":")).encode() + b"\\n")\n'
                    '                broker.stdin.flush()\n                return reply()',
                    '''                if command["verb"] != "enqueue":
                    broker.stdin.write(json.dumps(command, separators=(",", ":")).encode() + b"\\n")
                    broker.stdin.flush()
                    return reply()
                frame = json.dumps(command, separators=(",", ":")).encode() + b"\\n"
                self.enqueue_clock.mark("pipe_write_start")
                broker.stdin.write(frame)
                broker.stdin.flush()
                self.enqueue_clock.mark("pipe_write_done")
                result = reply()
                self.enqueue_clock.mark("caller_line_ready")
                return result''')
    return replace(stock, '''            batch = json.loads(path.read_text()) if self.engine == "mesh" else str(path)
            response = request({"verb": "enqueue", "batch": batch})
            log.write_text(response + "\\n")
            return self.read_receipt(response, log)''', '''            self.enqueue_clock = Clock()
            self.enqueue_clock.mark("input_start")
            data = path.read_text()
            self.enqueue_clock.mark("input_read")
            batch = json.loads(data)
            del data
            self.enqueue_clock.mark("input_loaded")
            response = request({"verb": "enqueue", "batch": batch})
            log.write_text(response + "\\n")
            self.enqueue_clock.mark("caller_receipt_check_start")
            result = self.read_receipt(response, log)
            self.enqueue_clock.mark("caller_receipt_checked")
            self.cluster.caller_stage = self.enqueue_clock.finish()
            return result''')


if __name__ == "__main__":
    stock, target = map(Path, sys.argv[1:])
    (target / "split_arm.py").write_text(split_source((stock / "split_arm.py").read_text()))
    (target / "combined-run").write_text(launch_source((stock / "combined-run").read_text()))
    (target / "combined-run").chmod(0o755)
