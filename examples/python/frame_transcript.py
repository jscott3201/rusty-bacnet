"""Observe a virtual device's socket while it answers and optionally reads a peer.

Run with --help. Ctrl-C stops the transcript and closes the endpoint.
Raw octets are unredacted; this example does not apply CLI password masking.
"""
import argparse
import asyncio
from contextlib import suppress
import json

from rusty_bacnet import BipEndpoint, ObjectIdentifier, ObjectType, PropertyIdentifier, decode_frame


async def print_frames(frames):
    async for frame in frames:
        summary = decode_frame(frame.octets, link=frame.link)
        print(json.dumps({
            "timestamp": frame.timestamp,
            "direction": frame.direction,
            "local_address": frame.local_address,
            "peer": frame.peer,
            "original_source": summary.original_source,
            "routed_source": summary.routed_source,
            "routed_destination": summary.routed_destination,
            "pdu": summary.pdu,
            "service": summary.service,
            "invoke_id": summary.invoke_id,
            "problem": summary.problem,
            "parse_error": summary.parse_error,
            "missed": frames.missed,
            "octets": frame.octets.hex(),
        }), flush=True)


async def main(args):
    async with BipEndpoint(args.instance, interface=args.interface, port=args.port) as endpoint:
        frames = await endpoint.frames(capacity=256)
        observer = asyncio.create_task(print_frames(frames))
        try:
            print(json.dumps({"listening": await endpoint.local_address()}), flush=True)
            if args.target is not None:
                client = await endpoint.client()
                await client.read_property(
                    args.target, ObjectIdentifier(ObjectType.DEVICE, args.target_instance),
                    PropertyIdentifier.OBJECT_IDENTIFIER,
                )
            await observer
        finally:
            observer.cancel()
            with suppress(asyncio.CancelledError):
                await observer
            await frames.aclose()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--interface", default="127.0.0.1", help="local IPv4 address")
    parser.add_argument("--port", type=int, default=47808)
    parser.add_argument("--instance", type=int, default=1001)
    parser.add_argument("--target", help="optional peer IPv4 address:port to read")
    parser.add_argument("--target-instance", type=int)
    args = parser.parse_args()
    if args.target is not None and args.target_instance is None:
        parser.error("--target requires --target-instance")
    try:
        asyncio.run(main(args))
    except KeyboardInterrupt:
        pass
