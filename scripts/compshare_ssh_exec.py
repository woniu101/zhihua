"""Run a bounded command on an authorized CompShare instance over SSH."""

from __future__ import annotations

import argparse
import shlex
from pathlib import Path

import paramiko

from bootstrap_compshare_ssh import (
    PinnedFingerprintPolicy,
    invoke,
    load_credentials,
)


MAX_OUTPUT_BYTES = 256 * 1024


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("credential_file", type=Path)
    parser.add_argument("instance_id")
    parser.add_argument("private_key_file", type=Path)
    parser.add_argument("host_fingerprint")
    command_source = parser.add_mutually_exclusive_group(required=True)
    command_source.add_argument("--command")
    command_source.add_argument("--command-file", type=Path)
    args = parser.parse_args()

    public_key, private_key = load_credentials(args.credential_file)
    response = invoke(
        public_key,
        private_key,
        "DescribeCompShareInstance",
        Limit=100,
        Offset=0,
        WithoutGpu="true",
    )
    instance = next(
        item for item in response.get("UHostSet", []) if item.get("UHostId") == args.instance_id
    )
    if instance.get("State") != "Running":
        raise SystemExit(f"Instance is not running: {instance.get('State')}")

    login = shlex.split(instance["SshLoginCommand"])
    port = int(login[login.index("-p") + 1])
    user, host = login[-1].split("@", 1)

    client = paramiko.SSHClient()
    client.load_system_host_keys()
    client.set_missing_host_key_policy(
        PinnedFingerprintPolicy(args.host_fingerprint)
    )
    client.connect(
        host,
        port=port,
        username=user,
        key_filename=str(args.private_key_file),
        look_for_keys=False,
        allow_agent=False,
        timeout=15,
    )
    command = (
        args.command_file.read_text(encoding="utf-8")
        if args.command_file
        else args.command
    )
    _, stdout, stderr = client.exec_command(command, timeout=900)
    output = stdout.read(MAX_OUTPUT_BYTES + 1)
    error = stderr.read(MAX_OUTPUT_BYTES + 1)
    status = stdout.channel.recv_exit_status()
    client.close()

    if len(output) > MAX_OUTPUT_BYTES or len(error) > MAX_OUTPUT_BYTES:
        raise SystemExit("Remote command output exceeded the safety limit")
    if output:
        print(output.decode(errors="replace"), end="")
    if error:
        print(error.decode(errors="replace"), end="")
    raise SystemExit(status)


if __name__ == "__main__":
    main()
