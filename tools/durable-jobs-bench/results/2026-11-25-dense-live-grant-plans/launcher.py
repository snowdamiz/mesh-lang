"""Rewrite the probe entry point in the pinned public launcher."""

def rewrite(source, root, lane, module):
    for old, new, count in (
        ('source="$(cd "$here/../../../../.." && pwd -P)"', f'source="{root}"', 1),
        ('"$here/product_arm.py" "$@"', f'"$here/observe.py" {lane} {module} "$@"', 2),
    ):
        if source.count(old) != count:
            raise ValueError(f"launcher anchor changed: {old}")
        source = source.replace(old, new)
    return source
