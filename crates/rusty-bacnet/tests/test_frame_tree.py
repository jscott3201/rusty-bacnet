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

        def projection(letter):
            return [(node.label, node.span, node.device_text)
                    for node in walk(rb.decode_frame_tree(prefix + letter, "bip"))]
        one = projection(b"A")
        self.assertEqual(one, projection(b"B"))
        self.assertIn(("Password: (hidden)", (12, 15), False), one)


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
