"""Installed native GetAlarmSummary policy; independently encoded UDP requests."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer


class AlarmSummaryConstructorTests(unittest.TestCase):
    def test_defaults_keyword_only_stub_and_validation(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [("alarm_summary_max_objects", 4096),
                              ("alarm_summary_max_service_ack_bytes", 16384)]:
            parameter = signature.parameters[name]
            self.assertEqual(parameter.default, default)
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIn(f"{name}: int = {default}",
                          Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
            for transport in ["bip", "ipv6", "sc", "mstp"]:
                for value, error in [(0, ValueError), (-1, OverflowError),
                                     (1 << 200, OverflowError), (1.5, TypeError)]:
                    with self.subTest(name=name, transport=transport, value=value):
                        invalid: dict[str, Any] = {name: value}
                        with self.assertRaises(error):
                            BACnetServer(123, transport=transport, **invalid)
                positive: dict[str, Any] = {name: (1 << (8 * struct.calcsize("P"))) - 1}
                BACnetServer(123, transport=transport, **positive)


class AlarmSummaryNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_non_alarm_scan_work_abort_and_empty_ack_with_tiny_byte_budget(self):
        # The server adds a Device too: two database objects, no active alarms.
        for policy, expected in [
            ({"alarm_summary_max_objects": 1}, 9),
            ({"alarm_summary_max_objects": 2, "alarm_summary_max_service_ack_bytes": 1}, None),
            ({}, None),
            ({"alarm_summary_max_objects": 8192, "alarm_summary_max_service_ack_bytes": 32768}, None),
        ]:
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", **policy)
            server.add_analog_input(1, "AI")
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
            loop = asyncio.get_running_loop()
            try:
                await server.start()
                ip, port = (await server.local_address()).rsplit(":", 1)
                for segmented in [False, True]:
                    invoke = 2 if segmented else 1
                    npdu = b"\x01\x00" + bytes([2 if segmented else 0, 5, invoke, 3])
                    wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                    await loop.sock_sendto(sock, wire, (ip, int(port)))
                    reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
                    self.assertEqual(reply[:2], b"\x81\x0a")
                    self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
                    self.assertEqual(reply[6:], bytes([0x71, invoke, expected])
                                     if expected is not None else bytes([0x30, invoke, 3]))
                    async with asyncio.timeout(2):
                        while (await server.request_admission_counters())["confirmed_active"]:
                            await asyncio.sleep(0)
                    with self.assertRaises(BlockingIOError):
                        sock.recvfrom(4096)
            finally:
                await server.stop()
                sock.close()
