"""Install the local SSH public key on an authorized CompShare instance.

The platform password is used in memory for one connection and is never printed.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import json
import shlex
import time
import urllib.parse
import urllib.request
from pathlib import Path

import paramiko


API_URL = "https://api.compshare.cn"


class PinnedFingerprintPolicy(paramiko.MissingHostKeyPolicy):
    def __init__(self, expected: str) -> None:
        self.expected = expected

    def missing_host_key(self, client, hostname, key) -> None:
        digest = hashlib.sha256(key.asbytes()).digest()
        actual = "SHA256:" + base64.b64encode(digest).decode("ascii").rstrip("=")
        if actual != self.expected:
            raise paramiko.SSHException("SSH host key fingerprint mismatch")


def load_credentials(path: Path) -> tuple[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        separator = "：" if "：" in line else ":" if ":" in line else None
        if separator:
            key, value = line.split(separator, 1)
            values[key.strip()] = value.strip()
    return (
        next(value for key, value in values.items() if "公钥" in key),
        next(value for key, value in values.items() if "私钥" in key),
    )


def invoke(public_key: str, private_key: str, action: str, **values: object) -> dict:
    parameters = {
        "Action": action,
        "PublicKey": public_key,
        **{key: str(value) for key, value in values.items()},
    }
    source = "".join(key + parameters[key] for key in sorted(parameters)) + private_key
    parameters["Signature"] = hashlib.sha1(source.encode()).hexdigest()
    request = urllib.request.Request(
        API_URL,
        urllib.parse.urlencode(parameters).encode(),
        headers={"U-Timestamp-Ms": str(int(time.time() * 1000))},
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("credential_file", type=Path)
    parser.add_argument("instance_id")
    parser.add_argument("public_key_file", type=Path)
    parser.add_argument(
        "--host-fingerprint",
        help="Expected SHA-256 host key fingerprint for a new endpoint",
    )
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
    command = shlex.split(instance["SshLoginCommand"])
    port = int(command[command.index("-p") + 1])
    user, host = command[-1].split("@", 1)
    password = base64.b64decode(instance["Password"]).decode()
    local_key = args.public_key_file.read_text(encoding="utf-8").strip()

    client = paramiko.SSHClient()
    client.load_system_host_keys()
    if args.host_fingerprint:
        client.set_missing_host_key_policy(
            PinnedFingerprintPolicy(args.host_fingerprint)
        )
    else:
        client.set_missing_host_key_policy(paramiko.RejectPolicy())
    client.connect(
        host,
        port=port,
        username=user,
        password=password,
        look_for_keys=False,
        allow_agent=False,
        timeout=15,
    )
    safe_key = shlex.quote(local_key)
    _, stdout, stderr = client.exec_command(
        "mkdir -p ~/.ssh && chmod 700 ~/.ssh && "
        f"grep -qxF {safe_key} ~/.ssh/authorized_keys 2>/dev/null || echo {safe_key} >> ~/.ssh/authorized_keys; "
        "chmod 600 ~/.ssh/authorized_keys"
    )
    status = stdout.channel.recv_exit_status()
    error = stderr.read().decode(errors="replace").strip()
    client.close()
    if status != 0:
        raise SystemExit(f"Failed to install SSH key: {error}")
    print(f"SSH public key installed for {user}@{host}:{port}")


if __name__ == "__main__":
    main()
