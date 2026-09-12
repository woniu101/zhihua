"""Read-only CompShare API probe for development diagnostics.

The credential file stays outside the repository and is never printed.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import time
import urllib.parse
import urllib.request
from pathlib import Path


API_URL = "https://api.compshare.cn"


def load_credentials(path: Path) -> tuple[str, str]:
    values: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        separator = "：" if "：" in line else ":" if ":" in line else None
        if separator:
            key, value = line.split(separator, 1)
            values[key.strip()] = value.strip()
    public_key = next(value for key, value in values.items() if "公钥" in key)
    private_key = next(value for key, value in values.items() if "私钥" in key)
    return public_key, private_key


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
    parser.add_argument(
        "--create-cn-wlcb-01",
        action="store_true",
        help="Create one authorized 5090 development instance after a live capacity check.",
    )
    args = parser.parse_args()
    public_key, private_key = load_credentials(args.credential_file)
    balance = invoke(public_key, private_key, "GetBalance")
    account = balance.get("AccountInfo", {})
    print(
        json.dumps(
            {
                "balance": True,
                "amount": account.get("Amount"),
                "available": account.get("AmountAvailable"),
                "free": account.get("AmountFree"),
                "freeze": account.get("AmountFreeze"),
            },
            ensure_ascii=False,
        )
    )
    response = invoke(
        public_key,
        private_key,
        "DescribeCompShareInstance",
        Limit=100,
        Offset=0,
        WithoutGpu="true",
    )
    if response.get("RetCode") != 0:
        raise SystemExit(f"CompShare API error {response.get('RetCode')}: {response.get('Message')}")
    fields = (
        "UHostId",
        "Name",
        "Region",
        "Zone",
        "State",
        "Gpu",
        "GpuType",
        "Cpu",
        "Memory",
        "CompShareImageId",
        "ChargeType",
        "SshLoginCommand",
    )
    instances = response.get("UHostSet", [])
    for instance in instances:
        print(json.dumps({field: instance.get(field) for field in fields}, ensure_ascii=False))

    image_id = next(
        (instance.get("CompShareImageId") for instance in instances if instance.get("CompShareImageId")),
        None,
    )
    if not image_id:
        return
    zones_response = invoke(
        public_key,
        private_key,
        "DescribeCompShareSupportZone",
        Region="cn-wlcb",
    )
    if zones_response.get("RetCode") != 0:
        raise SystemExit(
            f"CompShare zone API error {zones_response.get('RetCode')}: {zones_response.get('Message')}"
        )
    creation_spec: dict | None = None
    for zone in zones_response.get("ZoneInfo", []):
        capacity = invoke(
            public_key,
            private_key,
            "CheckCompShareResourceCapacity",
            Region=zone["Region"],
            Zone=zone["Zone"],
            GpuType="5090",
            MachineType="G",
            MinimalCpuPlatform="Auto",
            CompShareImageId=image_id,
            ChargeType="Postpay",
            **{
                "Disks.0.IsBoot": "true",
                "Disks.0.Type": "CLOUD_SSD",
                "Disks.0.Size": 100,
            },
        )
        available = [
            spec
            for spec in capacity.get("Specs", [])
            if spec.get("Gpu") == 1 and spec.get("ResourceEnough")
        ]
        prices = []
        for spec in available:
            price = invoke(
                public_key,
                private_key,
                "GetCompShareInstancePrice",
                Region=zone["Region"],
                Zone=zone["Zone"],
                GpuType="5090",
                Gpu=1,
                Cpu=spec["Cpu"],
                Memory=spec["Mem"] * 1024,
                ChargeType="Postpay",
                CompShareImageId=image_id,
                **{
                    "Disks.0.IsBoot": "true",
                    "Disks.0.Type": "CLOUD_SSD",
                    "Disks.0.Size": 100,
                },
            )
            prices.append(
                {
                    "cpu": spec["Cpu"],
                    "memoryGb": spec["Mem"],
                    "details": price.get("PriceDetails", []),
                    "retCode": price.get("RetCode"),
                    "message": price.get("Message"),
                }
            )
        print(
            json.dumps(
                {
                    "capacity": True,
                    "region": zone["Region"],
                    "zone": zone["Zone"],
                    "description": zone.get("Describe"),
                    "retCode": capacity.get("RetCode"),
                    "message": capacity.get("Message"),
                    "availableOneGpuSpecs": available,
                    "prices": prices,
                },
                ensure_ascii=False,
            )
        )
        if (
            args.create_cn_wlcb_01
            and zone["Region"] == "cn-wlcb"
            and zone["Zone"] == "cn-wlcb-01"
            and available
        ):
            creation_spec = available[0]

    if args.create_cn_wlcb_01:
        if not creation_spec:
            raise SystemExit("No authorized 1x5090 capacity is currently available in cn-wlcb-01")
        created = invoke(
            public_key,
            private_key,
            "CreateCompShareInstance",
            Region="cn-wlcb",
            Zone="cn-wlcb-01",
            MachineType="G",
            MinimalCpuPlatform="Auto",
            CompShareImageId=image_id,
            GPU=1,
            GpuType="5090",
            CPU=creation_spec["Cpu"],
            Memory=creation_spec["Mem"] * 1024,
            ChargeType="Postpay",
            MaxCount=1,
            Name=f"zhihua-dev-{datetime.datetime.now().strftime('%Y%m%d-%H%M')}",
            **{
                "Disks.0.IsBoot": "true",
                "Disks.0.Type": "CLOUD_SSD",
                "Disks.0.Size": 100,
            },
        )
        print(
            json.dumps(
                {
                    "created": created.get("RetCode") == 0,
                    "retCode": created.get("RetCode"),
                    "message": created.get("Message"),
                    "instanceIds": created.get("UHostIds", []),
                    "warnings": created.get("Warning", []),
                },
                ensure_ascii=False,
            )
        )


if __name__ == "__main__":
    main()
