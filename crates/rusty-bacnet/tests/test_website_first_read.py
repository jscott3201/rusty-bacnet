"""Execute the Python first-read guide's exact script with an installed extension.

The normal binding unittest discovery includes these checks. To select them from
repository root: python -m unittest discover -s crates/rusty-bacnet/tests \
    -p test_website_first_read.py -v
"""
import asyncio
from pathlib import Path
import sys
import tempfile
import unittest


GUIDE = (Path(__file__).resolve().parents[3]
         / "website/src/content/docs/start/python.md")


def displayed_first_read(document: bytes) -> bytes:
    """Extract this page's sole Python fence without changing its script bytes."""
    lines = document.splitlines(keepends=True)
    starts = [index for index, line in enumerate(lines)
              if line.rstrip(b"\r\n") == b"```python"]
    if len(starts) != 1:
        raise ValueError("first-read guide must contain exactly one Python fence")
    start = starts[0] + 1
    for end in range(start, len(lines)):
        if lines[end].rstrip(b"\r\n") == b"```":
            source = b"".join(lines[start:end])
            if not source.strip():
                raise ValueError("first-read Python fence is empty")
            return source
    raise ValueError("first-read Python fence is not closed")


class FirstReadExtractionTests(unittest.TestCase):
    def test_preserves_script_bytes_and_ignores_other_languages(self):
        source = b"\r\n# caf\xc3\xa9\r\nprint('value')  \r\n\r\n"
        document = (b"```sh\necho before\n```\n```python\r\n" + source
                    + b"```\r\n```sh\necho after\n```\n")
        self.assertEqual(displayed_first_read(document), source)

    def test_missing_python_fence_fails(self):
        with self.assertRaisesRegex(ValueError, "exactly one"):
            displayed_first_read(b"```sh\npython first_read.py\n```\n")

    def test_multiple_python_fences_fail(self):
        for second in (b"```python\npass\n```\n", b"```python\npass\n"):
            with self.subTest(second=second), self.assertRaisesRegex(ValueError, "exactly one"):
                displayed_first_read(b"```python\npass\n```\n" + second)

    def test_unclosed_python_fence_fails(self):
        with self.assertRaisesRegex(ValueError, "not closed"):
            displayed_first_read(b"```python\nprint('unfinished')\n")

    def test_empty_python_fence_fails(self):
        with self.assertRaisesRegex(ValueError, "empty"):
            displayed_first_read(b"```python\n\n```\n")


class WebsiteFirstReadTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        from rusty_bacnet import BACnetServer

        directory = tempfile.TemporaryDirectory(prefix="bacnet-first-read-")
        self.addCleanup(directory.cleanup)
        self.script = Path(directory.name) / "first_read.py"
        self.script.write_bytes(displayed_first_read(GUIDE.read_bytes()))
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
