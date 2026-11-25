"""Run the Python first-read guide's program with an installed extension.

The page imports website/examples/python/first_read.py as it is (#2648), so
this runs that file. The normal binding unittest discovery includes these
checks. To select them from the repository root:
python -m unittest discover -s crates/rusty-bacnet/tests \
    -p test_website_first_read.py -v
"""
import asyncio
from pathlib import Path
import sys
import unittest


PROGRAM = (Path(__file__).resolve().parents[3]
           / "website/examples/python/first_read.py")


class WebsiteFirstReadTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        from rusty_bacnet import BACnetServer

        self.script = PROGRAM
        self.server = BACnetServer(
            device_instance=1234, device_name="First-read documentation test",
            interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
        )
        self.addAsyncCleanup(self.stop_server)
        self.server.add_analog_input(1, "Default point", units=64, present_value=72.5)
        self.server.add_analog_input(2, "Selected point", units=62, present_value=-12.25)
        await asyncio.wait_for(self.server.start(), timeout=15)
        self.address = await self.server.local_address()
        host, separator, port = self.address.rpartition(":")
        self.assertEqual((host, separator), ("127.0.0.1", ":"))
        self.assertTrue(port.isdecimal() and 1 <= int(port) <= 65535, self.address)

    async def stop_server(self):
        await asyncio.wait_for(self.server.stop(), timeout=15)

    async def run_sample(self, *arguments):
        # -I retains this interpreter's installed package while ignoring user
        # PYTHONPATH/site customizations. The displayed script stays unmodified.
        creation = asyncio.create_task(asyncio.create_subprocess_exec(
            sys.executable, "-I", str(self.script), self.address, *arguments,
            cwd=self.script.parent, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        ))
        try:
            process = await asyncio.shield(creation)
            stdout, stderr = await asyncio.wait_for(process.communicate(), timeout=30)
            return process.returncode, stdout.decode(), stderr.decode()
        finally:
            # If cancelled during creation, acquire its handle before cleaning
            # up. Kill and drain on timeout/cancellation, then reap the child.
            process = await creation
            if process.returncode is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
            await process.communicate()

    async def test_default_instance_prints_typed_value_and_exits(self):
        code, out, err = await self.run_sample()
        self.assertEqual((code, out.splitlines(), err), (0, ["real: 72.5"], ""))

    async def test_instance_option_selects_a_different_point(self):
        code, out, err = await self.run_sample("--instance", "2")
        self.assertEqual((code, out.splitlines(), err), (0, ["real: -12.25"], ""))

    async def test_unknown_object_reports_failure_and_exits(self):
        for instance in ("0", "3", "4194303"):
            with self.subTest(instance=instance):
                code, out, err = await self.run_sample("--instance", instance)
                self.assertEqual((code, out), (1, ""), err)
                self.assertTrue(err.startswith("BACnet read failed: "), err)
                self.assertNotIn("Traceback", err)

    async def test_instance_bounds_are_argument_errors(self):
        for instance in ("-1", "4194304"):
            with self.subTest(instance=instance):
                code, out, err = await self.run_sample("--instance", instance)
                self.assertEqual((code, out), (2, ""), err)
                self.assertIn("--instance must be in 0..4194303", err)
                self.assertNotIn("Traceback", err)


if __name__ == "__main__":
    unittest.main()
