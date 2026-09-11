"""Zero-only receive policy through a fresh installed native build and real mTLS/WS.

These independent wire peers deliberately bypass local advertised defaults.
Positive serviceability floors and caller-mutated local limits are not tested here.
"""
from rusty_bacnet import BACnetClient, BACnetServer
import test_sc_accept_uuid as accept
import test_sc_peer_uuid as request
import test_sc_hub_mtls as mtls


class ZeroLimitsTests(mtls.MtlsFixture):
    # Reuse fixtures without inheriting or recollecting their test methods.
    frame = mtls.NodeIdentityMtlsTests.frame
    send_frame = mtls.NodeIdentityMtlsTests.send_frame
    node = mtls.NodeIdentityMtlsTests.node
    check_accept = accept.AcceptUuidTests.check_accept
    open_peer = request.PeerUuidTests.open_peer
    send = request.PeerUuidTests.send
    binary = request.PeerUuidTests.binary
    check_request = request.PeerUuidTests.check_request

    async def test_native_nodes_zero_limits_wait_silently_then_recover(self):
        for api in (BACnetClient, BACnetServer):
            for limits in (b"\0\0\x10\0", b"\x20\0\0\0", bytes(4)):
                with self.subTest(api=api.__name__, limits=limits):
                    await self.check_accept(api, True, limits)

    async def test_native_nodes_zero_limits_expire_without_connecting(self):
        for api in (BACnetClient, BACnetServer):
            with self.subTest(api=api.__name__):
                await self.check_accept(api, False, bytes(4))

    async def test_native_hub_zero_limits_preserve_known_uuid_owner_and_repeat(self):
        for limits in (b"\0\0\x05\xc4", b"\x05\xc4\0\0", bytes(4)):
            with self.subTest(limits=limits):
                await self.check_request(b"\x02\0\0\0\0\2" + mtls.SERVER_UUID + limits)
