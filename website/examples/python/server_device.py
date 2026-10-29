"""Serve a small device over BACnet/IP: an Analog Input whose value the
application supplies, and a Binary Output that peers command."""
import asyncio
import sys

from rusty_bacnet import (BACnetServer, ObjectIdentifier, ObjectType,
                          PropertyIdentifier, PropertyValue)

DEVICE_INSTANCE = 1001
TEMPERATURE = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
FAN = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)


async def main() -> int:
    if len(sys.argv) != 1:
        print("usage: server_device.py (press Enter or send EOF to stop)",
              file=sys.stderr)
        return 2
    # The server answers only requests sent to 127.0.0.1. For a real
    # network, give the interface's address and its subnet broadcast.
    server = BACnetServer(DEVICE_INSTANCE, "Example air handler",
                          interface="127.0.0.1", port=0,
                          broadcast_address="127.0.0.1")
    server.add_analog_input(1, "Supply air temperature", units=62)  # degrees C
    server.add_binary_output(1, "Supply fan")
    await server.start()
    try:
        # The application owns the input's value.
        await server.set_present_value_local(TEMPERATURE,
                                             PropertyValue.real(21.5))
        print(f"Serving device {DEVICE_INSTANCE} at "
              f"{await server.local_address()}")
        print("Press Enter to stop (or send EOF).", flush=True)
        stop = asyncio.ensure_future(asyncio.to_thread(sys.stdin.readline))
        fan_on = False
        while not stop.done():
            # Peers command the fan; the application acts on the value the
            # priority array settles on.
            command = await server.read_property(
                FAN, PropertyIdentifier.PRESENT_VALUE)
            on = command.value == 1
            if on != fan_on:
                fan_on = on
                reading = 19.0 if on else 21.5
                await server.set_present_value_local(
                    TEMPERATURE, PropertyValue.real(reading))
                print(f"Supply fan: {'on' if on else 'off'}", flush=True)
            await asyncio.wait({stop}, timeout=0.1)
        stop.result()
    finally:
        await server.stop()
    print("Stopped")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
