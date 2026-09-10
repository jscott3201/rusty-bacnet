"""Runs against the real installed extension, never an external network.

Run from website/: python -m unittest discover -s tests/python -v
"""
import contextlib
import importlib.util
import io
from pathlib import Path
import subprocess
from queue import Queue
from threading import Thread
import sys
import unittest

SCRIPT = Path(__file__).resolve().parents[2] / "examples/python/loopback_lab.py"
spec = importlib.util.spec_from_file_location("loopback_lab", SCRIPT)
lab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lab)


class LabGuards(unittest.TestCase):
    def test_defaults(self):
        args = lab.parse_args([])
        self.assertEqual(args.port, 0)
        self.assertFalse(args.serve)

    def test_no_remote_host_option(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            lab.parse_args(["--host", "192.0.2.1"])

    def test_port_bounds(self):
        for value in ["-1", "65536"]:
            with self.subTest(value=value), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                lab.parse_args(["--port", value])

    def test_duration_bounds(self):
        for value in ["0", "3601"]:
            with self.subTest(value=value), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                lab.parse_args(["--seconds", value])

    def test_reported_address_must_be_exact_loopback(self):
        self.assertEqual(lab.require_local_address("127.0.0.1:47809"), "127.0.0.1:47809")
        for bad in ["0.0.0.0:47809", "192.0.2.1:47809", "127.0.0.1:0", "127.0.0.1:65536", "::1:47809"]:
            with self.subTest(bad=bad), self.assertRaises(RuntimeError):
                lab.require_local_address(bad)


class RealExtensionLab(unittest.TestCase):
    def test_default_lab_reads_and_exits(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], text=True, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("real: 72.5", result.stdout)
        self.assertIn("PASS:", result.stdout)
        self.assertIn("Server stopped.", result.stdout)

    def test_serve_mode_is_bounded(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--serve", "--seconds", "1"], text=True, capture_output=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--interface 127.0.0.1 --port 0 read 127.0.0.1:", result.stdout)
        self.assertIn("Server stopped.", result.stdout)

    def test_real_client_can_read_serve_mode(self):
        import asyncio
        from rusty_bacnet import BACnetClient, ObjectIdentifier, ObjectType, PropertyIdentifier
        process = subprocess.Popen([sys.executable, "-u", str(SCRIPT), "--serve", "--seconds", "3"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            lines = Queue()
            Thread(target=lambda: lines.put(process.stdout.readline()), daemon=True).start()
            first = lines.get(timeout=10).strip()
            self.assertTrue(first.startswith("Example server: "), first)
            address = lab.require_local_address(first.removeprefix("Example server: "))
            async def read():
                async with BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1", apdu_timeout_ms=1000) as client:
                    value = await client.read_property(address, ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), PropertyIdentifier.PRESENT_VALUE)
                    self.assertEqual(value.value, 72.5)
                    units = await client.read_property(address, ObjectIdentifier(ObjectType.ANALOG_INPUT, 1), PropertyIdentifier.UNITS)
                    self.assertEqual(units.value, 64)  # degrees Fahrenheit, not 62 (Celsius)
            asyncio.run(asyncio.wait_for(read(), timeout=5))
            out, err = process.communicate(timeout=10)
            self.assertEqual(process.returncode, 0, err)
            self.assertIn("Server stopped.", out)
        finally:
            if process.poll() is None:
                process.kill()
            process.communicate()


if __name__ == "__main__":
    unittest.main()
