"""Replay the local Linux nobody identity controls from their retained output."""
import json


def check_local(path):
    accepted, replaced, leaf, ancestor = map(json.loads, path.read_text().splitlines())
    assert accepted["uid"] == accepted["gid"] == 65534 and accepted["canonical"] is True
    assert "must name the installed stage executable" in accepted["dotdot"]
    assert "digest mismatch" in accepted["wrong_hash"]
    assert all("launch identity changed" in accepted[key] for key in ("wrong_argv", "wrong_launch"))
    assert any(reason in replaced["replaced"] for reason in ("kernel executable differs", "replaced since launch"))
    assert all("noncanonical executable path" in row[key]
               for row, key in ((leaf, "symlink"), (ancestor, "ancestor")))
    return dict(uid=65534, canonical=True, rejected=["dotdot", "wrong_hash", "wrong_argv", "wrong_launch",
                                                      "replaced", "symlink", "ancestor"])
