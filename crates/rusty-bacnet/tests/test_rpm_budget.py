"""Installed-extension RPM budgets, with independently encoded UDP requests."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer


class RpmConstructorTests(unittest.TestCase):
    def test_defaults_keyword_only_stub_and_ranges(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [("rpm_max_result_elements", 256), ("rpm_max_service_ack_bytes", 16384)]:
            parameter = signature.parameters[name]
            self.assertEqual(parameter.default, default)
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            stub = Path(rusty_bacnet.__file__).with_suffix(".pyi")
            self.assertIn(f"{name}: int = {default}", stub.read_text())
            for transport in ["bip", "ipv6", "sc", "mstp"]:
                for value, error in [(0, ValueError), (-1, OverflowError), (1 << 200, OverflowError), (1.5, TypeError)]:
                    with self.subTest(name=name, transport=transport, value=value):
                        with self.assertRaises(error):
                            invalid: dict[str, Any] = {name: value}
                            BACnetServer(123, transport=transport, **invalid)
                # Configuration is not used as an allocation capacity or a semaphore limit.
                positive: dict[str, Any] = {name: (1 << (8 * struct.calcsize("P"))) - 1}
                BACnetServer(123, transport=transport, **positive,
                             sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")


class RpmNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_work_bytes_and_success_with_both_client_segment_capabilities(self):
        for policy, expected in [
            ({"rpm_max_result_elements": 1}, 9),  # OUT_OF_RESOURCES
            ({"rpm_max_service_ack_bytes": 7}, 1),  # BUFFER_OVERFLOW
            ({"rpm_max_result_elements": 2, "rpm_max_service_ack_bytes": 25}, None),
        ]:
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", **policy)
            server.add_analog_input(1, "AI", present_value=72.5)
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
            loop = asyncio.get_running_loop()
            try:
                await server.start()
                ip, port = (await server.local_address()).rsplit(":", 1)
                for segmented in [False, True]:
                    invoke = 2 if segmented else 1
                    # RPM request AI:1, two occurrences of Present_Value.
                    service = b"\x0c\x00\x00\x00\x01\x1e\x09\x55\x09\x55\x1f"
                    npdu = b"\x01\x00" + bytes([2 if segmented else 0, 5, invoke, 14]) + service
                    wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                    await loop.sock_sendto(sock, wire, (ip, int(port)))
                    reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
                    self.assertEqual(reply[:2], b"\x81\x0a")
                    self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
                    if expected is not None:
                        self.assertEqual(reply[6:], bytes([0x71, invoke, expected]))
                    else:
                        element = b"\x29\x55\x4e\x44" + struct.pack(">f", 72.5) + b"\x4f"
                        ack = b"\x0c\x00\x00\x00\x01\x1e" + element * 2 + b"\x1f"
                        self.assertEqual(reply[6:], bytes([0x30, invoke, 14]) + ack)
                    # Native completion, not a timing delay, separates invocations.
                    async with asyncio.timeout(2):
                        while (await server.request_admission_counters())["confirmed_active"]:
                            await asyncio.sleep(0)
                    with self.assertRaises(BlockingIOError):
                        sock.recvfrom(4096)
            finally:
                await server.stop()
                sock.close()
