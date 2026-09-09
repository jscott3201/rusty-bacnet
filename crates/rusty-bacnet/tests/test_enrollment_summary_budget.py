"""Installed native GetEnrollmentSummary limits and independent UDP vectors."""
import asyncio
import inspect
import socket
import struct
import unittest
from pathlib import Path
from typing import Any

import rusty_bacnet
from rusty_bacnet import BACnetServer


class EnrollmentSummaryConstructorTests(unittest.TestCase):
    def test_defaults_keyword_only_stub_and_positive_validation(self):
        signature = inspect.signature(BACnetServer)
        for name, default in [("enrollment_summary_max_objects", 4096),
                              ("enrollment_summary_max_service_ack_bytes", 16384)]:
            parameter = signature.parameters[name]
            self.assertEqual(parameter.default, default)
            self.assertEqual(parameter.kind, inspect.Parameter.KEYWORD_ONLY)
            self.assertIn(f"{name}: int = {default}",
                          Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text())
            for transport in ["bip", "ipv6", "sc", "mstp"]:
                for value, error in [(0, ValueError), (-1, OverflowError),
                                     (1 << 200, OverflowError), (1.5, TypeError)]:
                    with self.subTest(name=name, transport=transport, value=value):
                        with self.assertRaises(error):
                            BACnetServer(123, transport=transport, **{name: value})
                positive: dict[str, Any] = {name: (1 << (8 * struct.calcsize("P"))) - 1}
                BACnetServer(123, transport=transport, **positive,
                             sc_ca_cert="ca.pem", sc_client_cert="cert.pem", sc_client_key="key.pem")


class EnrollmentSummaryNativeTests(unittest.IsolatedAsyncioTestCase):
    async def test_complete_ack_or_abort_direct_and_routed(self):
        for candidate, policy, expected in [
            (False, {"enrollment_summary_max_objects": 1}, 9),
            (False, {"enrollment_summary_max_objects": 2,
                     "enrollment_summary_max_service_ack_bytes": 1}, None),
            (True, {"enrollment_summary_max_service_ack_bytes": 12}, 1),
            (True, {"enrollment_summary_max_service_ack_bytes": 13}, None),
            (True, {}, None),
        ]:
            server = BACnetServer(123, interface="127.0.0.1", port=0,
                                  broadcast_address="127.0.0.1", **policy)
            server.add_notification_class(0, "NC")
            if candidate:
                server.add_analog_input(1, "AI")
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", 0))
            sock.setblocking(False)
            loop = asyncio.get_running_loop()
            try:
                await server.start()
                ip, port = (await server.local_address()).rsplit(":", 1)
                for routed in [False, True]:
                    for segmented in [False, True]:
                        invoke = 2 if segmented else 1
                        header = b"\x01\x08\x00\x07\x01\x09" if routed else b"\x01\x00"
                        # Service4, acknowledgmentFilter context0 all(0).
                        npdu = header + bytes([2 if segmented else 0, 0, invoke, 4, 9, 0])
                        wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
                        await loop.sock_sendto(sock, wire, (ip, int(port)))
                        reply, _ = await asyncio.wait_for(loop.sock_recvfrom(sock, 4096), 2)
                        self.assertEqual(reply[:2], b"\x81\x0a")
                        self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
                        if routed:
                            self.assertEqual(reply[4:11], b"\x01\x20\x00\x07\x01\x09\xff")
                            apdu = reply[11:]
                        else:
                            apdu = reply[6:]
                        if expected is not None:
                            self.assertEqual(apdu, bytes([0x71, invoke, expected]))
                        else:
                            self.assertEqual(apdu[:3], bytes([0x30, invoke, 4]))
                            self.assertEqual(len(apdu[3:]), 13 if candidate else 0)
                        async with asyncio.timeout(2):
                            while (await server.request_admission_counters())["confirmed_active"]:
                                await asyncio.sleep(0)
                        with self.assertRaises(BlockingIOError):
                            sock.recvfrom(4096)
            finally:
                await server.stop()
                sock.close()
