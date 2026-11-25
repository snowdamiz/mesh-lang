"""Reject invalid or ambiguous keys at the source-format boundary."""
import json
import unittest

from replay import OID_MAX, corrected_attribute, corrected_catalog, key, oid, query_id


class Keys(unittest.TestCase):
    def test_boundary_values_and_preserved_query(self):
        dictionary = {"db": "1", "role": str(OID_MAX), "query": "-9223372036854775808"}
        hook = dict(dictionary, db=1, role=OID_MAX)
        self.assertEqual(key(dictionary, dictionary=True), key(hook, dictionary=False))
        self.assertEqual(key(hook, dictionary=False), (1, OID_MAX, "-9223372036854775808"))
        self.assertEqual(query_id("0"), "0")
        self.assertEqual(query_id("9223372036854775807"), "9223372036854775807")

    def test_invalid_oid_formats(self):
        for value in (0, -1, OID_MAX + 1, None, True, 1.0, "1"):
            with self.subTest(hook=value), self.assertRaises(ValueError):
                oid(value, dictionary=False)
        for value in (1, True, None, "0", "-1", str(OID_MAX + 1), "01", "+1", " 1",
                      "1\n", "1.0", "1e1", "１"):
            with self.subTest(dictionary=value), self.assertRaises(ValueError):
                oid(value, dictionary=True)
        for field in ("db", "role"):
            row = dict(slot=3, db=5, role=10, query="0")
            row[field] = str(row[field])
            with self.assertRaisesRegex(ValueError, "hook OID type"):
                corrected_attribute(json.dumps(row), {"rows": []}, True, "mesh", [])

    def test_invalid_query_ids(self):
        for value in (5, True, None, "-0", "01", "+1", "1 ", "-9223372036854775809", "9223372036854775808"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                query_id(value)

    def test_catalog_collision_rejected_and_roles_retained(self):
        row = {"db": "5", "role": "10", "query": "-19", "family": "grant"}
        with self.assertRaisesRegex(ValueError, "duplicate normalized dictionary key"):
            corrected_catalog({"rows": [row, dict(row, family="settle")]})
        with self.assertRaises(ValueError):
            corrected_catalog({"rows": [row, dict(row, role="010")]})
        source = {"rows": [row, dict(row, role="11"), dict(row, db="6")]}
        result = corrected_catalog(source)
        self.assertEqual([key(item, dictionary=False) for item in result["rows"]],
                         [(5, 10, "-19"), (5, 11, "-19"), (6, 10, "-19")])
        self.assertEqual(source["rows"][0]["role"], "10")


if __name__ == "__main__":
    unittest.main()
