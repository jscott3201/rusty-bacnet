"""Run the router guide's Python program (#1989).

The page imports website/examples/python/bip_router.py as it is, so this runs
that file: a two-port B/IP router on 127.0.0.1, a device behind it, and a
client that finds the router and reads the device through it. From the
repository root:
python -m unittest discover -s crates/rusty-bacnet/tests \
    -p test_website_router.py -v
"""
import asyncio
from pathlib import Path
import sys
import unittest

PROGRAM = (Path(__file__).resolve().parents[3]
           / "website/examples/python/bip_router.py")


class WebsiteRouterTests(unittest.IsolatedAsyncioTestCase):
    async def test_the_client_reads_the_device_through_the_router(self):
        # -I keeps the installed extension and ignores user site settings.
        process = await asyncio.create_subprocess_exec(
            sys.executable, "-I", str(PROGRAM),
            stdin=asyncio.subprocess.DEVNULL, stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        try:
            stdout, stderr = await asyncio.wait_for(process.communicate(), timeout=60)
        finally:
            if process.returncode is None:
                process.kill()
                await process.communicate()
        self.assertEqual((process.returncode, stderr.decode()), (0, ""))
        self.assertEqual(stdout.decode().splitlines(), [
            "Router started: networks 1 and 5",
            "Router to network 5 found, reaching networks [5]",
            "Device 1005 found on network 5",
            "Read through the router: 8.5",
            "Stopped",
        ])


if __name__ == "__main__":
    unittest.main()
