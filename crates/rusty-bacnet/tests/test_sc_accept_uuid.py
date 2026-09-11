"""Installed native client/server silently reject nil Accept after real mTLS/WS."""
import asyncio
import base64
import hashlib
import ssl

from rusty_bacnet import BACnetClient, BACnetServer, BacnetError
import test_sc_hub_mtls as mtls


class AcceptUuidTests(mtls.MtlsFixture):
    # Reuse the existing bounded independent wire and certificate fixtures,
    # without inheriting (and collecting) the other class's test methods.
    frame = mtls.NodeIdentityMtlsTests.frame
    send_frame = mtls.NodeIdentityMtlsTests.send_frame
    node = mtls.NodeIdentityMtlsTests.node

    async def check_accept(self, api, recover):
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_3
        context.load_cert_chain(self.path("hub.pem"), self.path("hub.key"))
        context.load_verify_locations(self.path("site.pem"))
        context.verify_mode = ssl.CERT_REQUIRED
        tasks = []
        nil_checked = asyncio.Event()
        release_valid = asyncio.Event()
        loop = asyncio.get_running_loop()
        outcome = loop.create_future()
        uuid = mtls.SERVER_UUID if api is BACnetServer else mtls.CLIENT_UUID
        vmac = b"\x02\0\0\0\0\x04" if api is BACnetServer else b"\x02\0\0\0\0\x02"

        async def peer(reader, writer):
            tasks.append(asyncio.current_task())
            try:
                tls = writer.get_extra_info("ssl_object")
                self.assertEqual(tls.version(), "TLSv1.3")
                self.assertTrue(tls.getpeercert(binary_form=True))
                request = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 3)
                headers = dict(line.split(b":", 1) for line in request.split(b"\r\n")[1:] if b":" in line)
                key = next(value.strip() for name, value in headers.items()
                           if name.lower() == b"sec-websocket-key")
                accept = base64.b64encode(hashlib.sha1(
                    key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
                writer.write(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                             b"Connection: Upgrade\r\nSec-WebSocket-Accept: " + accept +
                             b"\r\nSec-WebSocket-Protocol: hub.bsc.bacnet.org\r\n\r\n")
                await asyncio.wait_for(writer.drain(), 3)
                opcode, request = await self.frame(reader, True)
                self.assertEqual(opcode, 2)
                self.assertEqual(request, b"\x06\0\0\1" + vmac + uuid + b"\x16\x49\x05\xc4")
                # Independent base-2020 AB.2.11 bytes, not a product codec.
                nil = b"\x07\0\0\1" + b"\x22" * 6 + bytes(16) + b"\x20\0\x10\0"
                for wire in (nil, nil[:2] + b"\x33\x44" + nil[4:],
                             b"\x07\x02\0\1\x5e" + nil[4:]):
                    await self.send_frame(writer, wire)
                    with self.assertRaises(asyncio.TimeoutError):
                        await asyncio.wait_for(reader.read(1), 0.05)
                nil_checked.set()
                if recover:
                    await asyncio.wait_for(release_valid.wait(), 3)
                    await self.send_frame(writer, nil[:10] + b"\xff" * 16 + nil[26:])
                    # The public operation may emit I-Am after startup. Drain
                    # only after the valid Accept, until explicit owned stop.
                    async def drain():
                        while await reader.read(4096):
                            pass
                    await asyncio.wait_for(drain(), 5)
                else:
                    # No response is allowed even at native connect timeout.
                    # Keep this peer alive beyond the default 10-second budget.
                    self.assertEqual(await asyncio.wait_for(reader.read(1), 15), b"")
                outcome.set_result(None)
            except Exception as error:
                outcome.set_exception(error)
            finally:
                writer.close()
                await asyncio.wait_for(writer.wait_closed(), 3)

        listener = await asyncio.start_server(peer, "127.0.0.1", 0, ssl=context)
        node = self.node(api, f"wss://localhost:{listener.sockets[0].getsockname()[1]}", uuid, vmac)
        # PyO3 returns an awaitable Future, not necessarily a coroutine.
        started = asyncio.ensure_future(node.start() if api is BACnetServer else node.__aenter__())
        try:
            await asyncio.wait_for(nil_checked.wait(), 3)
            self.assertFalse(started.done(), "nil Accept completed native SC startup")
            if recover:
                release_valid.set()
                await asyncio.wait_for(started, 5)
                await self.stop_server(node)
            else:
                with self.assertRaisesRegex(BacnetError, "[Tt]imeout|timed out"):
                    await asyncio.wait_for(started, 15)
            await asyncio.wait_for(outcome, 5)
        finally:
            if not started.done():
                started.cancel()
            await asyncio.gather(started, return_exceptions=True)
            await self.stop_server(node)
            listener.close()
            await listener.wait_closed()
            for task in tasks:
                if not task.done() and not outcome.done():
                    task.cancel()
            results = await asyncio.wait_for(
                asyncio.gather(*tasks, return_exceptions=True), 3)
            if outcome.done() and not outcome.cancelled():
                outcome.exception()  # Consume even when a prior assertion failed.
            for result in results:
                if isinstance(result, Exception):
                    raise result  # Observe close errors, not intentional cancellation.

    async def test_native_nodes_wait_silently_then_accept_valid_uuid(self):
        for api in (BACnetClient, BACnetServer):
            with self.subTest(api=api.__name__):
                await self.check_accept(api, True)

    async def test_native_nodes_nil_accept_expires_without_connecting(self):
        for api in (BACnetClient, BACnetServer):
            with self.subTest(api=api.__name__):
                await self.check_accept(api, False)
