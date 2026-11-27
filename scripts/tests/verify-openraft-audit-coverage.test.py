#!/usr/bin/env python3
import importlib.util
import shutil
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "openraft_audit_coverage", ROOT / "scripts/verify-openraft-audit-coverage.py"
)
coverage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(coverage)


class CoverageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        paths = [
            "Cargo.toml", "Cargo.lock", "fuzz/Cargo.toml", "fuzz/Cargo.lock",
            "scripts/openraft-upstream-audit.lock",
        ]
        for name in ("openraft", "openraft-macros"):
            paths.extend(f"third_party/{name}/{file}" for file in (
                "Cargo.toml", ".cargo_vcs_info.json",
            ))
        for relative in paths:
            destination = self.root / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, destination)

    def test_current_identities(self):
        coverage.verify(self.root)

    def test_incomplete_upstream_identities(self):
        path = self.root / "scripts/openraft-upstream-audit.lock"
        path.write_text(path.read_text().rsplit("[[package]]", 1)[0])
        with self.assertRaises(SystemExit):
            coverage.verify(self.root)

    def test_wrong_upstream_checksum(self):
        path = self.root / "scripts/openraft-upstream-audit.lock"
        path.write_text(path.read_text().replace(
            "80d35e2f60cdf9bcfc39a020966091017c6dc2a4b43b355a22ca3e76106f4a0a", "0" * 64,
        ))
        with self.assertRaises(SystemExit):
            coverage.verify(self.root)

    def test_wrong_vendor_version(self):
        path = self.root / "third_party/openraft/Cargo.toml"
        path.write_text(path.read_text().replace('version = "0.9.24"', 'version = "0.9.25"', 1))
        with self.assertRaises(SystemExit):
            coverage.verify(self.root)

    def test_wrong_workspace_resolution(self):
        path = self.root / "fuzz/Cargo.lock"
        path.write_text(path.read_text().replace('name = "openraft"\nversion = "0.9.24"',
                                                 'name = "openraft"\nversion = "0.9.25"'))
        with self.assertRaises(SystemExit):
            coverage.verify(self.root)


if __name__ == "__main__":
    unittest.main()
