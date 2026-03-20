"""Scan BACnet network for BBMDs, read BDTs and FDTs, discover routers."""
import asyncio
from rusty_bacnet import BACnetClient


async def main():
    async with BACnetClient(port=47808, broadcast_address="255.255.255.255") as client:
        # Discover devices
        await client.who_is()
        await asyncio.sleep(3)
        devices = await client.discovered_devices()
        print(f"Found {len(devices)} devices")

        # Check each device for BBMD status by trying to read BDT
        for dev in devices:
            mac = dev.mac_address
            if len(mac) == 6:
                addr = f"{mac[0]}.{mac[1]}.{mac[2]}.{mac[3]}:{int.from_bytes(mac[4:6], 'big')}"
            else:
                continue

            try:
                bdt = await client.read_bdt(addr)
                print(f"\nBBMD found at {addr} — BDT has {len(bdt)} entries:")
                for entry in bdt:
                    print(f"  {entry.ip}:{entry.port} mask={entry.mask}")

                # Read FDT from confirmed BBMD
                fdt = await client.read_fdt(addr)
                print(f"  FDT has {len(fdt)} entries:")
                for entry in fdt:
                    print(
                        f"    {entry.ip}:{entry.port} "
                        f"ttl={entry.ttl} remaining={entry.seconds_remaining}"
                    )
            except Exception:
                pass  # Not a BBMD

        # Discover routers
        routers = await client.who_is_router_to_network()
        print(f"\nFound {len(routers)} routers:")
        for r in routers:
            print(f"  {r.address} serves networks: {r.networks}")

        await client.stop()


if __name__ == "__main__":
    asyncio.run(main())
