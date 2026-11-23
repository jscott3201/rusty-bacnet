"""An endpoint segments both ways by default (#2617).

The endpoint classes' ``segmentation`` defaults to ``SEGMENTED_BOTH``: the
I-Am says so, and a read whose answer is longer than one APDU is answered in
segments. ``Segmentation.NO_SEGMENTATION`` opts out, and the same read then
draws the Abort a device that can't segment sends. ``apdu_segment_timeout_ms``
is the Device's APDU_Segment_Timeout.
"""
import unittest

from rusty_bacnet import (
    AbortReason, BACnetClient, BacnetAbortError, BipEndpoint, ObjectIdentifier, ObjectType,
    PropertyIdentifier, Segmentation,
)

INSTANCE = 2618
DEVICE = ObjectIdentifier(ObjectType.DEVICE, INSTANCE)
AI1 = ObjectIdentifier(ObjectType.ANALOG_INPUT, 1)
# Longer than the 1476-octet APDU both sides take.
LONG_NAME = "segmented-" * 200


def endpoint(**keywords):
    """A B/IP endpoint on loopback whose AI-1 has a long name."""
    owner = BipEndpoint(INSTANCE, interface="127.0.0.1", port=0,
                        broadcast_address="127.0.0.1", **keywords)
    owner.add_analog_input(1, LONG_NAME, present_value=3.0)
    return owner


def client():
    return BACnetClient(interface="127.0.0.1", port=0, broadcast_address="127.0.0.1",
                        apdu_timeout_ms=2000)


class EndpointSegmentationTests(unittest.IsolatedAsyncioTestCase):
    async def i_am_segmentation(self, reader, address):
        """The segmentation the endpoint's I-Am states."""
        await reader.discover(scope=address, low=INSTANCE, high=INSTANCE, wait=2)
        rows = [row for row in await reader.discovered_devices()
                if row.object_identifier == DEVICE]
        self.assertEqual(len(rows), 1)
        return rows[0].segmentation_supported

    async def test_the_default_segments_both_ways(self):
        async with endpoint() as owner, client() as reader:
            address = await owner.local_address()
            self.assertEqual(await self.i_am_segmentation(reader, address),
                             Segmentation.SEGMENTED_BOTH)
            name = await reader.read_property(address, AI1, PropertyIdentifier.OBJECT_NAME)
            self.assertEqual(name.value, LONG_NAME)
            server = await owner.server()
            timeout = await server.read_property(
                DEVICE, PropertyIdentifier.APDU_SEGMENT_TIMEOUT)
            self.assertEqual(timeout.value, 5000)

    async def test_no_segmentation_opts_out(self):
        async with endpoint(segmentation=Segmentation.NO_SEGMENTATION) as owner, \
                client() as reader:
            address = await owner.local_address()
            self.assertEqual(await self.i_am_segmentation(reader, address),
                             Segmentation.NO_SEGMENTATION)
            with self.assertRaises(BacnetAbortError) as raised:
                await reader.read_property(address, AI1, PropertyIdentifier.OBJECT_NAME)
            self.assertEqual(raised.exception.reason, AbortReason.SEGMENTATION_NOT_SUPPORTED)

    async def test_the_segment_timeout_is_the_devices(self):
        async with endpoint(apdu_segment_timeout_ms=7300) as owner:
            server = await owner.server()
            timeout = await server.read_property(
                DEVICE, PropertyIdentifier.APDU_SEGMENT_TIMEOUT)
            self.assertEqual(timeout.value, 7300)

    def test_a_zero_segment_timeout_needs_no_segmentation(self):
        with self.assertRaises(ValueError):
            endpoint(apdu_segment_timeout_ms=0)
        # Without segmentation the Device serves no segment timeout.
        endpoint(segmentation=Segmentation.NO_SEGMENTATION, apdu_segment_timeout_ms=0)


if __name__ == "__main__":
    unittest.main()
