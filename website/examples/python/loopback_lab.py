"""A bounded BACnet/IP learning lab on this machine only.

Requires rusty-bacnet==0.11.0 and Python >=3.11. No physical device is needed.
Default: create an example server, read its analog input, verify it, and stop.
--serve: keep that server available for directed CLI reads for a limited time.
Neither mode performs Who-Is discovery, a remote write, or a COV subscription.
"""
from __future__ import annotations

import argparse
import asyncio
from importlib.metadata import PackageNotFoundError, version
import sys

LOOPBACK = "127.0.0.1"
EXPECTED_VERSION = "0.11.0"


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serve", action="store_true", help="Keep the example server running for directed local reads")
    parser.add_argument("--seconds", type=int, default=300, help="Server duration in serve mode (1–3600; default 300)")
    parser.add_argument("--port", type=int, default=0, help="Server UDP port; 0 chooses an unused ephemeral port")
    args = parser.parse_args(argv)
    if not 0 <= args.port <= 65535:
        parser.error("--port must be in 0..65535")
    if not 1 <= args.seconds <= 3600:
        parser.error("--seconds must be in 1..3600")
    return args


def check_version() -> None:
    try:
        installed = version("rusty-bacnet")
    except PackageNotFoundError as exc:
        raise RuntimeError("Install rusty-bacnet==0.11.0 in this Python environment first.") from exc
    if installed != EXPECTED_VERSION:
        raise RuntimeError(f"This lab targets {EXPECTED_VERSION}; installed package is {installed}. Use a matching environment.")


def require_local_address(address: str) -> str:
    """Fail closed if the server reports an unexpected interface or port."""
    host, separator, raw_port = address.rpartition(":")
    if separator != ":" or host != LOOPBACK or not raw_port.isdecimal() or not 1 <= int(raw_port) <= 65535:
        raise RuntimeError(f"Expected a bound loopback endpoint, got {address!r}.")
    return address


async def run_lab(args: argparse.Namespace) -> None:
    check_version()
    # Deferred import means --help and argument checks do not need the extension.
    from rusty_bacnet import BACnetClient, BACnetServer, ObjectIdentifier, ObjectType, PropertyIdentifier

    server = BACnetServer(
        device_instance=1234,
        device_name="Local documentation lab",
        interface=LOOPBACK,
        port=args.port,
        broadcast_address=LOOPBACK,
    )
    server.add_analog_input(instance=1, name="Zone Temperature", units=64, present_value=72.5)
    try:
        await asyncio.wait_for(server.start(), timeout=5)
        address = require_local_address(await server.local_address())
        print(f"Example server: {address}", flush=True)
        if args.serve:
            print("Open another terminal. Substitute your installed executable for bacnet:", flush=True)
            print(f"bacnet --interface {LOOPBACK} --port 0 read {address} ai:1 pv", flush=True)
            print(f"Stops after {args.seconds} seconds, or press Ctrl+C.", flush=True)
            await asyncio.sleep(args.seconds)
        else:
            async with BACnetClient(
                interface=LOOPBACK, port=0, broadcast_address=LOOPBACK, apdu_timeout_ms=1500
            ) as client:
                value = await asyncio.wait_for(
                    client.read_property(
                        address, ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), PropertyIdentifier.PRESENT_VALUE
                    ),
                    timeout=5,
                )
                if value.tag != "real" or value.value != 72.5:
                    raise RuntimeError(f"Unexpected sample result: {value.tag}: {value.value}")
                print(f"Read ai:1 present-value: {value.tag}: {value.value}", flush=True)
                print("PASS: directed local read; no remote property writes.", flush=True)
    finally:
        await asyncio.wait_for(server.stop(), timeout=5)
        print("Server stopped.", flush=True)


def main() -> int:
    args = parse_args()
    try:
        asyncio.run(run_lab(args))
    except KeyboardInterrupt:
        print("Lab interrupted.", file=sys.stderr)
        return 130
    except Exception as exc:
        print(f"Lab failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
