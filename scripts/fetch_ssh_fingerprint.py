"""Read an SSH server key fingerprint without changing known_hosts."""

from __future__ import annotations

import argparse
import base64
import hashlib
import socket

import paramiko


def fetch_fingerprint(host: str, port: int) -> str:
    """Return the SHA-256 fingerprint used by the native tunnel verifier."""
    with socket.create_connection((host, port), timeout=15) as connection:
        transport = paramiko.Transport(connection)
        try:
            transport.start_client(timeout=15)
            key = transport.get_remote_server_key()
            digest = hashlib.sha256(key.asbytes()).digest()
            encoded = base64.b64encode(digest).decode("ascii").rstrip("=")
            return f"SHA256:{encoded}"
        finally:
            transport.close()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("host")
    parser.add_argument("port", type=int)
    args = parser.parse_args()
    print(fetch_fingerprint(args.host, args.port))


if __name__ == "__main__":
    main()
