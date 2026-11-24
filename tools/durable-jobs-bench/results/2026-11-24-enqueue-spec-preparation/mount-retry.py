"""Retry the frozen packet with unpadded block-device identity output."""
import argparse
from pathlib import Path
import subprocess

import controller

ID = controller.ID + "-mount-retry"
PACKET = controller.HERE / "request-mount-retry.json"
ORIGINAL_SHA = "5f9110eefc0de20c2bfa7ab58a06d2fe84945182171e66cad206341d818dd156"
ORIGINAL_SHELL = controller.HERE / "native.sh"
FAILED_SHA = "59869db34853933b239f1e2219cf7d431da006aa919635478511fabbfc8849d2"
ORIGINAL_GRANT = controller.grant
SSH = controller.run.ssh


def shell():
    original = ORIGINAL_SHELL.read_text()
    padded = '$(findmnt -no MAJ:MIN --target "$base")'
    assert original.count(padded) == 1
    return original.replace(padded, '$(findmnt -nro MAJ:MIN --target "$base")')


def ssh(args, **options):
    if list(args[:3]) == ["bash", "-c", ORIGINAL_SHELL.read_text()]:
        args = [*args[:2], shell(), *args[3:]]
    return SSH(args, **options)


def grant(kind, request_id):
    if (kind, request_id) == ("SETUP", controller.ID):
        request_id = ID
    ORIGINAL_GRANT(kind, request_id)


def check():
    controller.check()
    digest = controller.run.digest
    assert digest(controller.HERE / "request.json") == ORIGINAL_SHA
    assert digest(controller.HERE / "evidence/setup-de98a0588ecd/raw.tar.gz") == FAILED_SHA
    request = controller.run.read_json(PACKET)
    assert request["id"] == ID and request["cpus"] == [0, 1]
    assert request["local_sha256"] == {str(Path(__file__).relative_to(controller.ROOT)): digest(Path(__file__))}
    assert PACKET.read_bytes() == (controller.QUEUE / f"{ID}.json").read_bytes()
    subprocess.run(["bash", "-n"], input=shell(), text=True, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "setup", "timing", "broker"))
    parser.add_argument("--setup-id")
    parser.add_argument("--component-suffix")
    parser.add_argument("--profile", choices=("short", "large"))
    args = parser.parse_args()
    check()
    if args.action == "check":
        print("Original archives preserved and unpadded device check verified")
        return
    controller.grant, controller.run.ssh = grant, ssh
    controller.dispatch(args.action, args.setup_id, args.component_suffix, args.profile)


if __name__ == "__main__":
    main()
