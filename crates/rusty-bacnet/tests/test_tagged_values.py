"""Values with no typed form read as their tags (#2618).

A value that holds a context tag and has no typed form, a vendor's property
or a standard one such as Lighting_Command, reads as ``tagged``: ``value``
is a list of ``TagNode`` records, application values typed, context
primitives as their contents, constructed elements with their members, and
``octets`` are the octets exactly as served.
"""

from __future__ import annotations

import asyncio
import copy
import pickle
import socket
import unittest

from rusty_bacnet import (
    BACnetClient,
    ObjectIdentifier,
    ObjectType,
    PropertyIdentifier,
    PropertyValue,
    TagNode,
)

# [0] 2, [1] ( REAL 1.5, "hi", [2] ( TRUE ) ), Unsigned 7.
VENDOR = bytes.fromhex("0902" "1e" "443fc00000" "73006869" "2e112f" "1f" "2107")
VENDOR_OBJECT = ObjectIdentifier(ObjectType.from_raw(130), 1)
VENDOR_PROPERTY = PropertyIdentifier.from_raw(1000)


def frame(apdu: bytes) -> bytes:
    """A B/IP original-unicast frame carrying `apdu` with no NPDU options."""
    payload = b"\x01\x00" + apdu
    return b"\x81\x0a" + (len(payload) + 4).to_bytes(2, "big") + payload


class TaggedValueTests(unittest.TestCase):
    def test_a_vendor_value_is_a_tree_of_records(self) -> None:
        value = PropertyValue.tagged(VENDOR)
        self.assertEqual(value.tag, "tagged")
        self.assertEqual(value.octets, VENDOR)
        self.assertEqual(repr(value), f"PropertyValue.tagged(<{len(VENDOR)} bytes>)")
        first, second, third = value.value
        self.assertEqual(
            (first.tag_class, first.tag_number, first.constructed, first.value, first.octets),
            ("context", 0, False, b"\x02", b"\x09\x02"),
        )
        self.assertEqual(repr(first), "<TagNode context 0, 1 octet>")
        self.assertTrue(second.constructed)
        self.assertEqual(second.octets, VENDOR[2:16])
        real, text, inner = second.value
        self.assertEqual((real.tag_class, real.tag_number), ("application", 4))
        self.assertEqual(real.value, PropertyValue.real(1.5))
        self.assertEqual(text.value.value, "hi")
        self.assertEqual(inner.value[0].value, PropertyValue.boolean(True))
        self.assertEqual(repr(second), "<TagNode context 1 constructed, 3 members>")
        self.assertEqual(repr(real), "<TagNode application 4 real 1.5>")
        self.assertEqual(repr(text), "<TagNode application 7 character_string 'hi'>")
        # A long value is cut short; contents are counted, never shown.
        long = PropertyValue.tagged(b"\x0e\x75\x29\x00" + b"x" * 40 + b"\x0f\x0d\x28" + bytes(40))
        constructed, context = long.value
        self.assertEqual(repr(constructed.value[0]),
                         "<TagNode application 7 character_string 'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx…>")
        self.assertEqual(repr(context), "<TagNode context 0, 40 octets>")
        self.assertEqual(third.value, PropertyValue.unsigned(7))
        # Each node's octets are the value's, end to end.
        self.assertEqual(b"".join(node.octets for node in value.value), VENDOR)

    def test_tagged_values_compare_by_octets_and_copy(self) -> None:
        value = PropertyValue.tagged(VENDOR)
        self.assertEqual(value, PropertyValue.tagged(VENDOR))
        self.assertEqual(hash(value), hash(PropertyValue.tagged(VENDOR)))
        # The same octets as application data are another value.
        self.assertNotEqual(value, PropertyValue.application_data(VENDOR))
        self.assertEqual(value.octets, PropertyValue.application_data(VENDOR).octets)
        for copied in (copy.copy(value), copy.deepcopy(value), pickle.loads(pickle.dumps(value))):
            self.assertEqual(copied, value)
            self.assertEqual(copied.value, value.value)
        for node in value.value:
            self.assertIsInstance(node, TagNode)
            self.assertEqual(pickle.loads(pickle.dumps(node)), node)
            self.assertEqual(hash(copy.deepcopy(node)), hash(node))

    def test_tagged_items_of_a_list_stay_tagged(self) -> None:
        tagged = PropertyValue.tagged(VENDOR)
        for items in ([tagged], [tagged, PropertyValue.unsigned(1)]):
            with self.subTest(items=items):
                listed = PropertyValue.list(items)
                self.assertEqual(listed.tag, "list")
                self.assertEqual(listed.value[0], tagged.value)
                self.assertEqual(listed.octets, b"".join(item.octets for item in items))
                for copied in (copy.deepcopy(listed), pickle.loads(pickle.dumps(listed))):
                    self.assertEqual(copied, listed)
                    self.assertEqual(copied.value[0], tagged.value)
        # Beside plain application data the items can't say which they are:
        # all read as octets.
        plain = PropertyValue.list([tagged, PropertyValue.application_data(b"\x09\x01")])
        self.assertEqual(plain.value, [VENDOR, b"\x09\x01"])
        self.assertNotEqual(PropertyValue.list([tagged]),
                            PropertyValue.list([PropertyValue.application_data(VENDOR)]))

    def test_octets_that_dont_read_as_tags_raise(self) -> None:
        for octets in (
            b"\x1e\x21\x01",  # never closed
            b"\x09\x01\x2f",  # a closing tag with nothing open
            b"\x0e" * 33 + b"\x0f" * 33,  # nested past 32
            b"\x09\x01\x75\x05\x03\x00\x00\x00\x41",  # a UCS-4 string
            b"\x44\x3f",  # cut short
            b"\x00" * 10_001,  # past the 10,000-element bound
        ):
            with self.subTest(octets=octets.hex()), self.assertRaises(ValueError):
                PropertyValue.tagged(octets)


class VendorReadTests(unittest.IsolatedAsyncioTestCase):
    async def read(self, octets: bytes) -> PropertyValue:
        """A ReadProperty of a vendor property from a peer that answers
        with `octets` as the value."""
        loop = asyncio.get_running_loop()
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
            peer.bind(("127.0.0.1", 0))
            peer.setblocking(False)
            address = f"127.0.0.1:{peer.getsockname()[1]}"
            async with BACnetClient(interface="127.0.0.1", port=0, apdu_timeout_ms=2000) as client:
                read = asyncio.ensure_future(
                    client.read_property(address, VENDOR_OBJECT, VENDOR_PROPERTY))
                wire, remote = await asyncio.wait_for(loop.sock_recvfrom(peer, 2048), 2)
                ack = bytes([0x30, wire[8], 12]) + wire[10:] + b"\x3e" + octets + b"\x3f"
                await loop.sock_sendto(peer, frame(ack), remote)
                return await asyncio.wait_for(read, 2)

    async def test_a_vendor_property_reads_as_its_tags(self) -> None:
        value = await self.read(VENDOR)
        self.assertEqual(value, PropertyValue.tagged(VENDOR))
        self.assertEqual([node.tag_number for node in value.value], [0, 1, 2])

    async def test_application_values_alone_keep_their_shape(self) -> None:
        # No context tag: the values themselves, as before.
        value = await self.read(bytes.fromhex("2107" "443fc00000"))
        self.assertEqual(value, PropertyValue.list([PropertyValue.unsigned(7),
                                                    PropertyValue.real(1.5)]))

    async def test_what_the_tree_cant_show_keeps_its_octets(self) -> None:
        # A context field beside an ENUMERATED past 32 bits.
        octets = bytes.fromhex("0901" "950501000000" "00")
        value = await self.read(octets)
        self.assertEqual((value.tag, value.value), ("application_data", octets))


if __name__ == "__main__":
    unittest.main()
