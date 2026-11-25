"""Change only the entry point and source locator of the frozen launcher."""
from layout import LANES


def rewrite(source, root, lane):
    if lane not in LANES:
        raise ValueError("unknown same-page lane")
    for old, new, count in (
        ('source="$(cd "$here/../../../../.." && pwd -P)"', f'source="{root}"', 1),
        ('"$here/product_arm.py" "$@"', f'"$here/observe.py" {lane} "$@"', 2),
    ):
        if source.count(old) != count:
            raise ValueError(f"launcher anchor changed: {old}")
        source = source.replace(old, new)
    return source
