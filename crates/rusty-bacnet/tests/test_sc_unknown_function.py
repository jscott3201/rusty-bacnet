"""Raw authenticated fake hub -> installed native NODE, not the hub fallback.

Wire/admission and explicit fresh starts only: Rust owns real-clock deadline,
held-write and automatic fresh-only recovery evidence. No native 60s claim.
"""
import test_sc_hub_mtls as mtls
import test_sc_rejection_deadline as deadline


class UnknownFunctionNodeTests(mtls.MtlsFixture):
    frame = deadline.RejectionDeadlineTests.frame
    send_frame = deadline.RejectionDeadlineTests.send_frame
    node = deadline.RejectionDeadlineTests.node
    binary = deadline.RejectionDeadlineTests.binary
    exercise_rejections = deadline.RejectionDeadlineTests.exercise_rejections

    async def test_unknown_function_fake_hub_to_native_node_and_fresh_read_property(self):
        source = b"\x22" * 6
        local = b"\x02\0\0\0\0\x04"
        cases = []
        for raw in (0x0D, 0x42, 0xFF):
            for message_id in (b"\0\0", b"\xff\xff"):
                prefix = b"\0\0" + message_id
                result = bytes([raw, 1, 0, 0, 7, 0, 0x8F])
                cases.append((bytes([raw, 0]) + message_id, prefix + result))
                # Unknown wins over both MU options and absent/present payload.
                for payload in (b"", b"\x01\x04\x00\x03\x11\x0c\x0c\0\0\0\0\x19\x55"):
                    body = b"\xe2\0\0\x1f\x7e\0\1\xbb" + payload
                    cases.append((bytes([raw, 11]) + message_id + source + body,
                                  b"\0\4" + message_id + source + result))
                for origin in (b"\0" * 6, b"\xff" * 6):
                    cases.append((bytes([raw, 8]) + message_id + origin, None))
                for destination in (b"\xff" * 6, local, b"\x44" * 6, b"\0" * 6):
                    cases.append((bytes([raw, 12]) + message_id + source + destination, None))
        # Known-but-unhandled functions are not newly classified as Unknown.
        for raw in (2, 3, 4, 5, 6, 7, 12):
            cases.append((bytes([raw, 0, 0x22, 0x33]), None))
        # ResultFor unknown is not an unknown outer function (ACK stays healthy).
        cases.append((b"\0\0\x22\x33\x42\0", None))
        cases.append((b"\x0a\0\x22\x33", b"\x0b\0\x22\x33"))
        await self.exercise_rejections(cases)
