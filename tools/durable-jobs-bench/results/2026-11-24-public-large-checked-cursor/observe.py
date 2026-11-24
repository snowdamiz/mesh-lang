"""Install one diagnostic lane in the original public large native launcher."""
import os
from pathlib import Path
import runpy
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import backend
from diagnostic import physical, install_thin

MODE = sys.argv.pop(1)
assert MODE in ("trace", "thin")
assert sys.argv[1:3] == ["mesh", "large"]
assert os.environ["PRODUCT_TOPOLOGY"] == "stock" and os.environ["PRODUCT_COMPOSITION"] == "product-bulk"
assert os.environ["PRODUCT_STORAGE"] in ("default", "external-4m")
arm = backend.arm


class ObservedCluster(backend.Cluster):
    def configure_tls(self):
        super().configure_tls()
        if MODE == "trace":
            with (self.data / "postgresql.conf").open("a") as stream:
                stream.write("log_line_prefix = '%m [%p] %l %u %d | '\n")

    def reconcile_trial(self, engine, namespace, enqueued_ids, handler_attempts, commits):
        result = super().reconcile_trial(engine, namespace, enqueued_ids, handler_attempts, commits)
        rows = physical(lambda q: self.sql("mesh", q), namespace)
        arm.h.write_json(self.evidence / "large-physical.json", rows)
        return result


class ObservedSubject(backend.Subject):
    def call(self, action, env, log):
        result = super().call(action, env, log)
        if action != "migrate":
            return result
        receipt = {"mode": MODE}
        if MODE == "thin":
            receipt.update(install_thin(lambda q: self.cluster.sql("mesh", q), env["JOBS_PARITY_NAMESPACE"]))
        else:
            settings = dict(log_min_duration_statement="0", log_parameter_max_length="0",
                            log_parameter_max_length_on_error="0")
            self.cluster.sql("mesh", "\n".join(
                f"ALTER ROLE parity_subject IN DATABASE mesh SET {k} = '{v}';" for k,v in settings.items()))
            receipt["settings"] = settings
        arm.h.write_json(self.cluster.evidence / "large-diagnostic.json", receipt)
        return result


backend.Cluster = ObservedCluster
backend.Subject = ObservedSubject
runpy.run_path(str(HERE / "product_arm.py"), run_name="__main__")
