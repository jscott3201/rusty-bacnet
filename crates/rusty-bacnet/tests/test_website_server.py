"""Run the server guide's Python device program and talk to it as a peer (#1989).

The page imports website/examples/python/server_device.py as it is, so this
runs that file: a client reads the input the program supplies, commands its
output, and checks the program saw the command. From the repository root:
python -m unittest discover -s crates/rusty-bacnet/tests \
    -p test_website_server.py -v
"""
import asyncio
from pathlib import Path
import socket
import sys
import unittest

PROGRAM = (Path(__file__).resolve().parents[3]
           / "website/examples/python/server_device.py")
READY = "Press Enter to stop (or send EOF)."


class WebsiteServerDeviceTests(unittest.IsolatedAsyncioTestCase):
    async def start_program(self, *arguments):
        # -I keeps the installed extension and ignores user site settings.
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-I", str(PROGRAM), *arguments,
            stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        self.addAsyncCleanup(self.reap, process)
        return process

    async def reap(self, process):
        if process.returncode is None:
            try:
                process.kill()
            except ProcessLookupError:
                pass
        await process.communicate()

    async def line(self, process):
        raw = await asyncio.wait_for(process.stdout.readline(), timeout=30)
        # Windows ends the child's lines with CRLF.
        return raw.decode().rstrip("\r\n")

    async def ready(self, process):
        first = await self.line(process)
        self.assertTrue(first.startswith("Serving device 1001 at 127.0.0.1:"), first)
        self.assertEqual(await self.line(process), READY)
        return first.rsplit(" ", 1)[1]

    async def finish(self, process, data=b""):
        stdout, stderr = await asyncio.wait_for(process.communicate(data), timeout=30)
        return (process.returncode, stdout.decode().replace("\r\n", "\n"),
                stderr.decode().replace("\r\n", "\n"))

    async def test_a_peer_reads_the_input_and_commands_the_output(self):
        from rusty_bacnet import (BACnetClient, ObjectIdentifier, ObjectType,
                                  PropertyIdentifier, PropertyValue)
        temperature = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
        fan = ObjectIdentifier(ObjectType.BINARY_OUTPUT, 1)
        pv = PropertyIdentifier.PRESENT_VALUE
        process = await self.start_program()
        address = await self.ready(process)
        async with BACnetClient(interface="127.0.0.1", port=0,
                                broadcast_address="127.0.0.1") as client:
            self.assertEqual((await client.read_property(address, temperature, pv)).value, 21.5)
            name = await client.read_property(address, temperature,
                                              PropertyIdentifier.OBJECT_NAME)
            self.assertEqual(name.value, "Supply air temperature")
            device = await client.read_property(
                address, ObjectIdentifier(ObjectType.DEVICE, 1001),
                PropertyIdentifier.OBJECT_NAME)
            self.assertEqual(device.value, "Example air handler")
            # A command at priority 8 reaches the program, which answers by
            # changing the input; relinquishing it turns the fan back off.
            await client.write_property(address, fan, pv, PropertyValue.enumerated(1), priority=8)
            self.assertEqual(await self.line(process), "Supply fan: on")
            self.assertEqual((await client.read_property(address, temperature, pv)).value, 19.0)
            await client.write_property(address, fan, pv, PropertyValue.null(), priority=8)
            self.assertEqual(await self.line(process), "Supply fan: off")
            self.assertEqual((await client.read_property(address, temperature, pv)).value, 21.5)
        code, out, err = await self.finish(process, b"\n")
        self.assertEqual((code, out, err), (0, "Stopped\n", ""))
        # Stopped means the port is free again.
        host, port = address.rsplit(":", 1)
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind((host, int(port)))

    async def test_end_of_input_also_stops_the_server(self):
        process = await self.start_program()
        await self.ready(process)
        code, out, err = await self.finish(process)
        self.assertEqual((code, out, err), (0, "Stopped\n", ""))

    async def test_arguments_are_refused_before_a_socket_opens(self):
        process = await self.start_program("47808")
        code, out, err = await self.finish(process)
        self.assertEqual((code, out), (2, ""))
        self.assertIn("usage: server_device.py", err)


if __name__ == "__main__":
    unittest.main()
