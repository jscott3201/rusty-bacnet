"""Route between two BACnet/IP networks on one host: a router joins
networks 1 and 5, and a client on network 1 finds the router, then finds
and reads a device on network 5 through it."""
import asyncio
import sys

from rusty_bacnet import (BACnetClient, BACnetRouter, BACnetServer, Bbmd,
                          BipRouterPort, ForeignDevice, ObjectIdentifier,
                          ObjectType, PropertyIdentifier)

NEAR, FAR = 1, 5
DEVICE_INSTANCE = 1005


def router_port(network_number):
    """One router port on 127.0.0.1, on a port the OS picks. Every node here
    shares that address, so a B/IP broadcast would reach nobody else: each
    port is a BBMD, and the other nodes register with it as foreign devices
    to send and receive broadcasts."""
    return BipRouterPort(network_number, interface="127.0.0.1", port=0,
                         broadcast_address="127.0.0.1",
                         bbmd=Bbmd(accept_foreign_devices=True,
                                   alone_on_subnet=True))


async def registered(*nodes, seconds=2.0):
    """Broadcasts flow once every registration is accepted."""
    deadline = asyncio.get_running_loop().time() + seconds
    for node in nodes:
        while not (await node.foreign_registration())["registered"]:
            if asyncio.get_running_loop().time() > deadline:
                raise RuntimeError("no foreign-device registration")
            await asyncio.sleep(0.02)


async def main() -> int:
    async with BACnetRouter([router_port(NEAR), router_port(FAR)]) as router:
        near, far = await router.ports()
        print(f"Router started: networks {NEAR} and {FAR}")
        server = BACnetServer(DEVICE_INSTANCE, "Rooftop unit",
                              interface="127.0.0.1", port=0,
                              broadcast_address="127.0.0.1",
                              foreign_device=ForeignDevice(far.address, ttl=60))
        server.add_analog_input(1, "Outdoor air temperature", units=62,
                                present_value=8.5)
        await server.start()
        try:
            async with BACnetClient(
                    interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                    foreign_device=ForeignDevice(near.address, ttl=60)) as client:
                await registered(server, client)
                # Who-Is-Router-To-Network: which router reaches network 5?
                routers = await client.discover_routers(FAR, timeout_ms=2000)
                if [found.mac for found in routers] != [near.mac]:
                    raise RuntimeError(f"expected this router, heard {routers}")
                print(f"Router to network {FAR} found, "
                      f"reaching networks {routers[0].networks}")
                # A global Who-Is crosses the router; the I-Am names the
                # device's network.
                devices = await client.discover(timeout_ms=2000,
                                                low_limit=DEVICE_INSTANCE,
                                                high_limit=DEVICE_INSTANCE)
                [device] = [found for found in devices
                            if found.object_identifier.instance == DEVICE_INSTANCE]
                print(f"Device {DEVICE_INSTANCE} found on network "
                      f"{device.source_network}")
                # Reads by instance go through the router that answered.
                value = await client.read_property_from_device(
                    DEVICE_INSTANCE, ObjectIdentifier(ObjectType.ANALOG_INPUT, 1),
                    PropertyIdentifier.PRESENT_VALUE)
                print(f"Read through the router: {value.value}")
        finally:
            await server.stop()
    # Leaving the router's context awaited its cleanup: its sockets are closed.
    print("Stopped")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
