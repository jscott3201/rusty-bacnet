"""Detailed offline fields from native-emitted traffic and explicit link fixtures."""
import asyncio
import unittest

import rusty_bacnet as rb


def walk(nodes):
    for node in nodes:
        yield node
        yield from walk(node.children)


class FieldTreeTests(unittest.TestCase):
    def test_link_tokens_and_malformed_frames(self):
        for link in ("bip", "ipv6", "sc", "mstp"):
            with self.subTest(link=link):
                fields = rb.decode_frame_tree(b"\x00", link)
                self.assertIsInstance(fields, tuple)
                self.assertIsInstance(fields[0], rb.FieldNode)
                self.assertTrue(any("malformed" in node.label.lower() for node in walk(fields)))
                for node in walk(fields):
                    start, end = node.span
                    self.assertLessEqual(0, start)
                    self.assertLessEqual(start, end)
                    self.assertLessEqual(end, 1)
        with self.assertRaisesRegex(ValueError, "one of"):
            rb.decode_frame_tree(b"", "other")
        with self.assertRaises(TypeError):
            rb.decode_frame_tree(b"")

    def test_password_is_hidden_in_the_tree(self):
        prefix = bytes.fromhex("810a000f01040005011119002a00")
        one = rb.decode_frame_tree(prefix + b"A", "bip")
        two = rb.decode_frame_tree(prefix + b"B", "bip")
        self.assertEqual(one, two)
        self.assertEqual(hash(one), hash(two))
        hidden = [node for node in walk(one) if node.label == "Password: (hidden)"]
        self.assertEqual([(node.span, node.device_text) for node in hidden], [((12, 15), False)])
        # The span still leads to the password's octets in the input.
        self.assertEqual((prefix + b"A")[slice(*hidden[0].span)], b"\x2a\x00A")

    def test_mstp_cobs_password_is_hidden_and_its_crc_is_a_status(self):
        # The same DCC request with password "A" or "B" in a COBS-encoded
        # BACnet Extended Data Expecting Reply frame from station 3 to 7,
        # octets from the stack's own MS/TP encoder: the encoded data and
        # its Encoded CRC-32K both differ.
        one = bytes.fromhex("55ff200703000f1e565451505054444c577f57145059d553ac")
        two = bytes.fromhex("55ff200703000f1e565451505054444c577f5717505a288ec1")
        trees = [rb.decode_frame_tree(octets, "mstp") for octets in (one, two)]
        self.assertEqual(trees[0], trees[1])
        labels = {node.label: node.span for node in walk(trees[0])}
        # A field decoded out of the COBS data points at the encoded data.
        self.assertEqual(labels["Password: (hidden)"], (8, 20))
        self.assertEqual(labels["Data: 12 octets"], (8, 20))
        self.assertEqual(labels["Encoded CRC-32K: valid"], (20, 25))
        # A damaged CRC: the Data and CRC fields stay, with a status only,
        # and nothing is decoded from the data.
        bad = one[:-1] + bytes([one[-1] ^ 1])
        roots = rb.decode_frame_tree(bad, "mstp")
        self.assertEqual(len(roots), 1)
        self.assertEqual(
            [(node.label, node.span) for node in roots[0].children[6:]],
            [("Data: 12 octets", (8, 20)), ("Encoded CRC-32K: mismatch", (20, 25)),
             ("malformed: data CRC mismatch", (0, 25))])

    def test_nodes_print_copy_and_pickle_by_value(self):
        import copy
        import pickle
        roots = rb.decode_frame_tree(bytes.fromhex("810a000f01040005011119002a0041"), "bip")
        self.assertEqual(repr(roots[0]),
                         "<FieldNode 'BVLL: original-unicast-npdu' 0..4, 3 children>")
        self.assertEqual(pickle.loads(pickle.dumps(roots)), roots)
        self.assertEqual(copy.deepcopy(roots), roots)
        self.assertEqual(len({roots[2], copy.copy(roots[2])}), 1)

    def test_input_past_the_largest_frame_is_refused(self):
        for link, limit in (("bip", 65535), ("ipv6", 65535), ("sc", 65535), ("mstp", 2053)):
            with self.subTest(link=link):
                rb.decode_frame_tree(bytes(limit), link)
                with self.assertRaisesRegex(ValueError, "longer than"):
                    rb.decode_frame_tree(bytes(limit + 1), link)


class CapturedFieldTreeTests(unittest.IsolatedAsyncioTestCase):
    async def test_captured_read_has_named_fields_and_marks_device_text(self):
        owner = rb.BipEndpoint(2633, interface="127.0.0.1", port=0,
                               broadcast_address="127.0.0.1")
        owner.add_analog_input(1, "Test sensor")
        async with owner:
            frames = await owner.frames()
            try:
                async with rb.BACnetClient(interface="127.0.0.1", port=0) as peer:
                    result = await peer.read_property(
                        await owner.local_address(), rb.ObjectIdentifier(rb.ObjectType.ANALOG_INPUT, 1),
                        rb.PropertyIdentifier.OBJECT_NAME,
                    )
                self.assertEqual(result.value, "Test sensor")
                seen = set()
                async with asyncio.timeout(10):
                    async for frame in frames:
                        summary = rb.decode_frame(frame.octets, frame.link)
                        if summary.service != "read-property":
                            continue
                        roots = rb.decode_frame_tree(frame.octets, frame.link)
                        self.assertEqual(len(roots), 3)
                        nodes = list(walk(roots))
                        for node in nodes:
                            self.assertIsInstance(node.children, tuple)
                            self.assertIsInstance(node.device_text, bool)
                            self.assertTrue(0 <= node.span[0] <= node.span[1] <= len(frame.octets))
                        by_label = {node.label: node for node in nodes}
                        obj = by_label["Object: analog-input 1"]
                        prop = by_label["Property: object-name"]
                        self.assertEqual(frame.octets[slice(*obj.span)], bytes.fromhex("0c00000001"))
                        self.assertEqual(frame.octets[slice(*prop.span)], bytes.fromhex("194d"))
                        self.assertFalse(prop.device_text)
                        with self.assertRaises(AttributeError):
                            prop.label = "changed"
                        if frame.direction == "outbound":
                            self.assertTrue(any(node.device_text and "Test sensor" in node.label
                                                for node in nodes))
                        seen.add(frame.direction)
                        if seen == {"inbound", "outbound"}:
                            break
                self.assertEqual(seen, {"inbound", "outbound"})
            finally:
                await frames.aclose()
