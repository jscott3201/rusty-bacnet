"""Offline Rust header summaries and same-socket trap transcripts."""
import asyncio
import socket
import unittest

from rusty_bacnet import (
    BACnetClient, BipEndpoint, FrameSummary, ObjectIdentifier, ObjectType,
    PropertyIdentifier, decode_frame,
)

# Fixed wire fixtures, decoded only by the Rust implementation under test.
REQUEST = bytes.fromhex("810a001101000005010c0c02000312194b")
FORWARDED = bytes.fromhex("8104001bc0000203bac001001000c40200007b2205c49103220104")
ROUTED = bytes.fromhex("810a001e01280008010600070105ff1000c40200007b2205c49103220104")


class FrameDecodeTests(unittest.TestCase):
    def test_request_summary_is_frozen_and_preserves_header_metadata(self):
        summary = decode_frame(REQUEST, "bip")
        self.assertIsInstance(summary, FrameSummary)
        self.assertEqual(summary.link_message, "original-unicast-npdu")
        self.assertEqual(summary.pdu, "Confirmed-Request")
        self.assertEqual(summary.invoke_id, 1)
        self.assertEqual(summary.service, "read-property")
        self.assertFalse(summary.broadcast)
        self.assertIsNone(summary.parse_error)
        self.assertIsNone(summary.problem)
        with self.assertRaises(AttributeError):
            summary.invoke_id = 3

    def test_forwarded_source_and_routed_addresses_have_distinct_fields(self):
        forwarded = decode_frame(FORWARDED, "bip")
        self.assertEqual(forwarded.original_source, "192.0.2.3:47808")
        self.assertIsNone(forwarded.link_source)
        self.assertEqual(forwarded.device, 123)
        self.assertTrue(forwarded.broadcast)
        routed = decode_frame(ROUTED, "bip")
        self.assertEqual((routed.snet, routed.dnet), (7, 8))
        self.assertEqual((routed.routed_source, routed.routed_destination), ("7:0x05", "8:0x06"))
        self.assertIsNone(routed.original_source)
        self.assertIsNone(routed.parse_error)

    def test_protocol_refusals_are_not_parse_errors(self):
        for octets, pdu in (
            ("810a000d010050090c91029120", "Error"),
            ("810a00090100600109", "Reject"),
            ("810a00090100710104", "Abort"),
            ("810000060030", None),
        ):
            with self.subTest(pdu=pdu):
                summary = decode_frame(bytes.fromhex(octets), "bip")
                self.assertEqual(summary.pdu, pdu)
                self.assertIsNotNone(summary.problem)
                self.assertIsNone(summary.parse_error)

    def test_malformed_link_npdu_and_apdu_return_partial_summaries(self):
        for octets in (b"\x81", bytes.fromhex("810a000500"), bytes.fromhex("810a00070100ff")):
            summary = decode_frame(octets, link="bip")
            self.assertIsNotNone(summary.parse_error)
            self.assertEqual(summary.parse_error, summary.problem)

    def test_link_is_required_and_a_wrong_link_is_a_parse_error(self):
        with self.assertRaises(TypeError):
            decode_frame(REQUEST)
        # B/IP octets read as BACnet/SC: 0x81 is no SC function.
        wrong = decode_frame(REQUEST, link="sc")
        self.assertEqual(wrong.link_message, "malformed")
        self.assertIsNotNone(wrong.parse_error)
        self.assertEqual(wrong.problem, wrong.parse_error)

    def test_summaries_compare_hash_and_print_by_value(self):
        one, again = decode_frame(REQUEST, "bip"), decode_frame(REQUEST, link="bip")
        self.assertEqual(one, again)
        self.assertEqual(hash(one), hash(again))
        self.assertEqual(len({one, again, decode_frame(b"\x81", "bip")}), 2)
        self.assertNotEqual(one, decode_frame(FORWARDED, "bip"))
        self.assertEqual(
            repr(one),
            'FrameSummary(link_message="original-unicast-npdu", pdu="Confirmed-Request", '
            'service="read-property", invoke_id=1)',
        )

    def test_iterator_link_names_and_explicit_names_select_the_same_decoder(self):
        for canonical, display in (("bip", "BACnet/IP"), ("bip6", "BACnet/IPv6"),
                                   ("sc", "BACnet/SC"), ("mstp", "MS/TP")):
            with self.subTest(link=display):
                expected = decode_frame(b"", link=canonical)
                actual = decode_frame(b"", link=display)
                self.assertEqual(actual.parse_error, expected.parse_error)
                self.assertIsNotNone(actual.parse_error)
        for unsupported in ("Ethernet", "BACnet Ethernet", "unknown", ""):
            with self.assertRaises(ValueError):
                decode_frame(REQUEST, link=unsupported)


class TrapTranscriptTests(unittest.IsolatedAsyncioTestCase):
    async def test_existing_socket_records_responses_malformed_and_forwarded_frames(self):
        async with BipEndpoint(715, interface="127.0.0.1", port=0) as endpoint:
            frames = await endpoint.frames()
            try:
                async with BACnetClient(interface="127.0.0.1", port=0) as client:
                    await client.read_property(
                        await endpoint.local_address(), ObjectIdentifier(ObjectType.DEVICE, 715),
                        PropertyIdentifier.OBJECT_IDENTIFIER,
                    )
                transcript = []
                for _ in range(2):
                    frame = await asyncio.wait_for(anext(frames), 10)
                    transcript.append((frame, decode_frame(frame.octets, link=frame.link)))
                self.assertEqual([row[1].pdu for row in transcript], ["Confirmed-Request", "Complex-ACK"])
                self.assertEqual([row[0].direction for row in transcript], ["inbound", "outbound"])
                self.assertEqual(transcript[0][1].invoke_id, transcript[1][1].invoke_id)

                with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as peer:
                    peer.bind(("127.0.0.1", 0))
                    peer.setblocking(False)
                    host, port = (await endpoint.local_address()).rsplit(":", 1)
                    for octets in (b"malformed transcript packet", FORWARDED):
                        await asyncio.get_running_loop().sock_sendto(peer, octets, (host, int(port)))
                        frame = await asyncio.wait_for(anext(frames), 10)
                        self.assertEqual(frame.octets, octets)
                        summary = decode_frame(frame.octets, link=frame.link)
                        if octets == FORWARDED:
                            self.assertEqual(summary.original_source, "192.0.2.3:47808")
                            self.assertEqual(frame.peer, "%s:%d" % peer.getsockname())
                            self.assertNotEqual(frame.peer, summary.original_source)
                            self.assertIsNone(summary.parse_error)
                        else:
                            self.assertIsNotNone(summary.parse_error)
            finally:
                await frames.aclose()


if __name__ == "__main__":
    unittest.main()
