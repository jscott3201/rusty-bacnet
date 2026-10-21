"""Real TLS admission exercise, run in a fresh single-worker native runtime.

Short frames in one TLS write keep a same-origin burst in the receive task's
ready path. A following SC heartbeat acknowledgment proves receive progress;
no sleeps, mocked counters, or native test hooks are used.
"""
import asyncio
import unittest

from rusty_bacnet import BACnetServer
from sc_client_fixture import ClientScFixture
import test_sc_hub_mtls as mtls

ORIGIN = b"\x02\0\0\0\0\4"
READ_BODY = b"\x0c\0\0\0\0\x19\x55"
ACK_BODY = READ_BODY + b"\x3e\x44\x42\x91\0\0\x3f"
ZERO = dict(control_rate_drops=0, fairness_drops=0, full_drops=0, closed_drops=0)


def frame(payload):
    assert len(payload) < 126
    return bytes([0x82, len(payload)]) + payload


def npdu(payload, message_id=1):
    # Encapsulated-NPDU with originating VMAC, as delivered by an SC Hub.
    return b"\x01\x08" + message_id.to_bytes(2, "big") + ORIGIN + payload


class ScNpduWireTests(ClientScFixture):
    async def exercise(self, server, options):
        ready, burst, processed, healthy, finish = [asyncio.Event() for _ in range(5)]

        async def receive_until(reader, predicate):
            for _ in range(40):
                opcode, value = await self.frame(reader, True)
                self.assertEqual(opcode, 2)
                if predicate(value):
                    return value
            self.fail("expected response missing from bounded received SC frames")

        async def peer(reader, writer, connect):
            await self.accept_sc(writer, connect)
            ready.set()
            for burst_index in range(2):
                await burst.wait()
                burst.clear()
                # Sixteen valid unconfirmed Who-Is NPDUs share the same origin.
                # Their combined size fits comfortably in one TLS record.
                heartbeat_id = (0x4455 + burst_index).to_bytes(2, "big")
                writer.write(b"".join(frame(npdu(b"\x01\x00\x10\x08", i + 10))
                                      for i in range(16)) + frame(b"\x0a\0" + heartbeat_id))
                await asyncio.wait_for(writer.drain(), 3)
                await receive_until(reader, lambda value: value == b"\x0b\0" + heartbeat_id)
                processed.set()
            await healthy.wait()
            if server:
                await self.send_frame(writer, npdu(b"\x01\x04\x00\x03\x71\x0c" + READ_BODY, 100))
                reply = await receive_until(reader, lambda value: b"\x30\x71\x0c" in value)
                self.assertTrue(reply.endswith(b"\x30\x71\x0c" + ACK_BODY), reply.hex())
            else:
                request = await receive_until(reader, lambda value: value.endswith(b"\x0c" + READ_BODY))
                self.assertEqual(request[:2], b"\x01\x04")
                self.assertEqual(request[4:10], ORIGIN)
                self.assertEqual(request[10:13], b"\x01\x04\x02")
                invoke = request[14:15]
                await self.send_frame(writer, npdu(b"\x01\x00\x30" + invoke + b"\x0c" + ACK_BODY, 100))
            finish.set()
            # Owned shutdown cancels this wait; keep the active transport alive
            # while the caller checks independent snapshots and stops it.
            await asyncio.Event().wait()

        url = await self.raw_peer(peer)
        if server:
            node = BACnetServer(
                3000, transport="sc", sc_hub=url, sc_vmac=b"\x02\0\0\0\0\2",
                sc_device_uuid=mtls.SERVER_UUID, sc_ca_cert=self.path("site.pem"),
                sc_client_cert=self.path("server.pem"), sc_client_key=self.path("server.key"),
                **options)
            self.addAsyncCleanup(self.stop_server, node)
        else:
            node = self.client(url, **options)
        for generation in range(2):
            # Every async start owns a fresh native transport and its counters.
            for event in [ready, burst, processed, healthy, finish]:
                event.clear()
            if server:
                node.add_analog_input(0, "Quota AI", 64, 72.5)
            await asyncio.wait_for(node.start() if server else node.__aenter__(), 5)
            await asyncio.wait_for(ready.wait(), 3)
            before = node.sc_npdu_drop_counts()
            self.assertIsInstance(before, asyncio.Future)
            self.assertEqual(await before, ZERO)
            previous = dict(ZERO)
            for burst_index in range(2):
                previous_copy = dict(previous)
                processed.clear()
                burst.set()
                await asyncio.wait_for(processed.wait(), 5)
                counts = await node.sc_npdu_drop_counts()
                self.assertEqual(set(counts), set(ZERO))
                self.assertTrue(all(type(value) is int for value in counts.values()))
                self.assertEqual(counts["full_drops"], 0)
                self.assertEqual(counts["closed_drops"], 0)
                if options.get("sc_npdu_per_origin_limit") == 1:
                    self.assertGreater(counts["fairness_drops"], previous["fairness_drops"])
                else:
                    self.assertEqual(counts, ZERO)
                self.assertEqual(previous, previous_copy)  # retained historical snapshot
                previous = counts
                print(f"{'server' if server else 'client'} options={options} generation={generation} "
                      f"burst={burst_index} counts={counts} heartbeat=ack", flush=True)
            healthy.set()
            if not server:
                await self.read_value(node)
            await asyncio.wait_for(finish.wait(), 5)
            after = await node.sc_npdu_drop_counts()
            self.assertEqual(after, counts)
            # Mutating either returned mapping cannot mutate another snapshot
            # or any native count. The mapping also survives owner shutdown.
            retained = dict(counts)
            counts["fairness_drops"] = -1
            self.assertEqual(await node.sc_npdu_drop_counts(), after)
            self.assertEqual(after, retained)
            await asyncio.wait_for(node.stop(), 5)
            with self.assertRaisesRegex(RuntimeError, "not started"):
                await node.sc_npdu_drop_counts()
            self.assertEqual(after, retained)
            print(f"{'server' if server else 'client'} options={options} generation={generation} "
                  f"counts={retained} heartbeat=ack ReadProperty=72.5", flush=True)

    async def test_client_quota_positive_default_and_larger_controls(self):
        for options in [{}, {"sc_npdu_per_origin_limit": None},
                        {"sc_npdu_per_origin_limit": 1}, {"sc_npdu_per_origin_limit": 32}]:
            with self.subTest(options=options):
                await self.exercise(False, options)

    async def test_server_quota_positive_default_and_larger_controls(self):
        for options in [{}, {"sc_npdu_per_origin_limit": None},
                        {"sc_npdu_per_origin_limit": 1}, {"sc_npdu_per_origin_limit": 32}]:
            with self.subTest(options=options):
                await self.exercise(True, options)


if __name__ == "__main__":
    unittest.main()
