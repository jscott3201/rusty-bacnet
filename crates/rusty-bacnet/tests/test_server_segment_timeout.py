"""Full-server segment timing/declarations with independently encoded UDP APDUs."""
import asyncio
import inspect
from pathlib import Path
import socket
import unittest

import rusty_bacnet
from rusty_bacnet import BACnetServer, Segmentation


class SegmentConstructorTests(unittest.TestCase):
    def test_keywords_defaults_and_invalid_values_precede_transport_setup(self):
        signature = inspect.signature(BACnetServer)
        for name in ("segmentation_supported", "apdu_segment_timeout_ms"):
            self.assertEqual(signature.parameters[name].kind, inspect.Parameter.KEYWORD_ONLY)
        self.assertEqual(signature.parameters["apdu_segment_timeout_ms"].default, 5000)
        stub = Path(rusty_bacnet.__file__).with_suffix(".pyi").read_text()
        self.assertIn("segmentation_supported: Segmentation = Segmentation.NONE", stub)
        self.assertIn("apdu_segment_timeout_ms: int = 5000", stub)
        for transport in ("bip", "ipv6", "sc", "mstp"):
            for mode in (Segmentation.TRANSMIT, Segmentation.RECEIVE, Segmentation.BOTH):
                with self.subTest(transport=transport, mode=mode):
                    with self.assertRaisesRegex(ValueError, "must be positive"):
                        BACnetServer(123, transport=transport, segmentation_supported=mode,
                                     apdu_segment_timeout_ms=0)
            with self.assertRaisesRegex(ValueError, "invalid segmentation"):
                BACnetServer(123, transport=transport, segmentation_supported=Segmentation.from_raw(64))
            for value, error in ((-1, OverflowError), (1 << 100, OverflowError),
                                 ((1 << 64) - 1, ValueError), (1.5, TypeError)):
                with self.subTest(transport=transport, value=value):
                    with self.assertRaises(error):
                        BACnetServer(123, transport=transport, apdu_segment_timeout_ms=value)
        for invalid in (None, 0, "BOTH"):
            with self.assertRaises(TypeError):
                BACnetServer(123, segmentation_supported=invalid)
        BACnetServer(123, apdu_segment_timeout_ms=0)  # Inactive under NONE.


def read_property(property_id):
    # Context0 Device:123, context1 one-octet property identifier.
    return b"\x0c\x02\x00\x00\x7b\x19" + bytes([property_id])


class SegmentWireTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.setblocking(False)
        self.loop = asyncio.get_running_loop()

    async def asyncTearDown(self):
        self.sock.close()

    async def exchange(self, address, apdu):
        npdu = b"\x01\x04" + apdu
        wire = b"\x81\x0a" + (len(npdu) + 4).to_bytes(2, "big") + npdu
        await self.loop.sock_sendto(self.sock, wire, address)
        reply, _ = await asyncio.wait_for(self.loop.sock_recvfrom(self.sock, 4096), 2)
        self.assertEqual(reply[:2], b"\x81\x0a")
        self.assertEqual(int.from_bytes(reply[2:4], "big"), len(reply))
        apdu = reply[6:]
        expected_control = 4 if apdu[0] & 0xF8 == 0x38 else 0
        self.assertEqual(reply[4:6], bytes([1, expected_control]))
        return apdu

    async def test_four_modes_property_and_runtime_directions(self):
        for mode in (Segmentation.NONE, Segmentation.TRANSMIT, Segmentation.RECEIVE, Segmentation.BOTH):
            with self.subTest(mode=mode):
                server = BACnetServer(123, device_name="long-device-name-" * 10,
                                      interface="127.0.0.1", port=0,
                                      broadcast_address="127.0.0.1",
                                      segmentation_supported=mode, apdu_segment_timeout_ms=7300)
                try:
                    await server.start()
                    host, port = (await server.local_address()).rsplit(":", 1)
                    address = (host, int(port))
                    for prop, encoded in ((107, bytes([0x91, mode.to_raw()])), (11, b"\x22\x17\x70"), (73, b"\x21\x03")):
                        service = read_property(prop)
                        reply = await self.exchange(address, b"\x00\x05\x01\x0c" + service)
                        self.assertEqual(reply, b"\x30\x01\x0c" + service + b"\x3e" + encoded + b"\x3f")
                    service = read_property(10)
                    reply = await self.exchange(address, b"\x00\x05\x02\x0c" + service)
                    if mode == Segmentation.NONE:
                        self.assertEqual(reply, b"\x50\x02\x0c\x91\x02\x91\x20")
                    else:
                        self.assertEqual(reply, b"\x30\x02\x0c" + service + b"\x3e\x22\x1c\x84\x3f")
                    # Request reassembly is real in RECEIVE/BOTH only.
                    service = read_property(11)
                    reply = await self.exchange(address, b"\x0e\x05\x03\x00\x01\x0c" + service[:4])
                    if mode in (Segmentation.RECEIVE, Segmentation.BOTH):
                        self.assertEqual(reply, b"\x41\x03\x00\x01")
                        reply = await self.exchange(address, b"\x0a\x05\x03\x01\x01\x0c" + service[4:])
                        self.assertEqual(reply, b"\x41\x03\x01\x01")
                        reply, _ = await asyncio.wait_for(self.loop.sock_recvfrom(self.sock, 4096), 2)
                        self.assertEqual(reply[6:], b"\x30\x03\x0c" + service + b"\x3e\x22\x17\x70\x3f")
                    else:
                        self.assertEqual(reply, b"\x71\x03\x04")
                    # A name too large for the peer's 50-byte APDU must segment
                    # only in TRANSMIT/BOTH, even though the client accepts it.
                    reply = await self.exchange(address, b"\x02\x00\x04\x0c" + read_property(77))
                    if mode in (Segmentation.TRANSMIT, Segmentation.BOTH):
                        self.assertEqual(reply[:5], b"\x3c\x04\x00\x01\x0c")
                    else:
                        self.assertEqual(reply, b"\x71\x04\x04")
                finally:
                    await server.stop()

    async def test_default_none_omits_segment_timeout_property(self):
        server = BACnetServer(123, interface="127.0.0.1", port=0, broadcast_address="127.0.0.1")
        try:
            await server.start()
            host, port = (await server.local_address()).rsplit(":", 1)
            reply = await self.exchange((host, int(port)), b"\x00\x05\x01\x0c" + read_property(10))
            self.assertEqual(reply, b"\x50\x01\x0c\x91\x02\x91\x20")
        finally:
            await server.stop()
