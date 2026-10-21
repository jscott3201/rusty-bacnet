"""Configured Python clients recover through real mutual TLS and SC admission."""
import asyncio
from pathlib import Path
import socket
import ssl

from rusty_bacnet import BacnetError, ScReconnect
from sc_client_fixture import ClientScFixture, mtls


class ClientScReconnectTests(ClientScFixture):
    async def test_same_client_reads_after_complete_hub_restart(self):
        hub, server = await self.serving_hub()
        relay = await self.relay(await hub.url())
        # The connector owns typed TLS policy after entry. Removing the source
        # files then proves retries do not silently reload application paths.
        retained = {}
        for key, name in [("sc_ca_cert", "site.pem"), ("sc_client_cert", "client.pem"),
                          ("sc_client_key", "client.key")]:
            copy = self.root / f"retained-{name}"
            copy.write_bytes((self.root / name).read_bytes())
            retained[key] = str(copy)
        client = self.client(relay.url, **retained, reconnect=ScReconnect(
            initial_delay_ms=100, max_delay_ms=100, max_retries=10))
        self.assertIs(await asyncio.wait_for(client.__aenter__(), 5), client)
        first = await relay.next_connection()
        await self.read_value(client)
        for path in retained.values():
            Path(path).unlink()
        self.assertEqual((await hub.status())["client_count"], 2)
        print("initial SC ReadProperty=72.5; two authenticated/admitted hub nodes", flush=True)

        relay.pause()
        await self.stop_server(server)
        await self.stop_hub(hub)
        await relay.disconnect()
        second = await relay.next_connection()
        self.assertIsNot(first, second)
        replacement, _ = await self.serving_hub()
        relay.resume(await replacement.url())

        async def recovered_read():
            while True:
                try:
                    await self.read_value(client)
                    return
                except BacnetError:
                    await asyncio.sleep(0.02)
        await asyncio.wait_for(recovered_read(), 5)
        self.assertEqual((await replacement.status())["client_count"], 2)
        print("same client recovered: fresh TCP/mTLS/SC admission; ReadProperty=72.5", flush=True)

    async def test_omitted_none_and_zero_retry_policy_never_redial_after_loss(self):
        cases = []
        for name, options in [("omitted", {}), ("none", {"reconnect": None}),
                              ("zero", {"reconnect": ScReconnect(
                                  initial_delay_ms=50, max_delay_ms=50, max_retries=0)})]:
            with self.subTest(policy=name):
                hub, _ = await self.serving_hub()
                relay = await self.relay(await hub.url())
                client = self.client(relay.url, **options)
                await asyncio.wait_for(client.__aenter__(), 5)
                await relay.next_connection()
                await self.read_value(client)
                await relay.disconnect()
                cases.append((name, client, relay))

        async def no_redial(name, client, relay):
            # Omitted/None must also catch accidentally enabling the default
            # policy: its first jittered sleep is 10-15s. Observe both peers
            # concurrently for the full upper bound plus scheduling margin.
            window = 0.35 if name == "zero" else 15.5
            with self.assertRaises(asyncio.TimeoutError, msg=name):
                await asyncio.wait_for(relay.accepted.get(), window)
            with self.assertRaises(BacnetError):
                await self.read_value(client)
            await asyncio.wait_for(client.stop(), 5)

        await asyncio.gather(*(no_redial(*case) for case in cases))

    async def test_retry_count_spacing_and_wrong_ca_rejection(self):
        hub, _ = await self.serving_hub()
        relay = await self.relay(await hub.url())
        client = self.client(relay.url, reconnect=ScReconnect(
            initial_delay_ms=120, max_delay_ms=120, max_retries=2))
        await asyncio.wait_for(client.__aenter__(), 5)
        await relay.next_connection()
        await self.read_value(client)

        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            listener.listen()

            def reject_both():
                # Observe actual TLS alert rejection twice, independently of
                # Rust's error mapping or a failed application read.
                for _ in range(2):
                    mtls.NodeMtlsTests.rejected_tls_peer(
                        self, listener, "wrong", "site", ssl.TLSVersion.TLSv1_3)
            peer = asyncio.create_task(asyncio.to_thread(reject_both))
            try:
                relay.resume(f"wss://localhost:{listener.getsockname()[1]}")
                await relay.disconnect()
                first = await relay.next_connection()
                second = await relay.next_connection()
                # The independent TCP accept clock allows 10ms measurement
                # tolerance. Equal floor/cap removes random backoff variance.
                self.assertGreaterEqual(second.accepted_at - first.accepted_at, 0.110)
                await peer
                with self.assertRaises(asyncio.TimeoutError):
                    await asyncio.wait_for(relay.accepted.get(), 0.4)
                self.assertEqual(len(relay.connections), 3)
                with self.assertRaises(BacnetError):
                    await self.read_value(client)
            finally:
                await asyncio.wait_for(client.stop(), 5)
                await peer

    async def test_identity_and_configured_heartbeat_survive_fresh_connection(self):
        connected, heartbeat = [], asyncio.Queue()

        async def peer(reader, writer, connect):
            self.assertEqual(connect[4:10], b"\x02\0\0\0\0\2")
            self.assertEqual(connect[10:26], mtls.CLIENT_UUID)
            self.assertEqual(connect[26:], b"\x16\x49\x05\xc6")
            connected.append(connect)
            await self.accept_sc(writer, connect)
            started = asyncio.get_running_loop().time()
            # The interval's tick origin may precede or follow the first
            # handshake. The configured 3s heartbeat arrives within 8s; the
            # default 30s heartbeat cannot satisfy this bounded observation.
            while True:
                header = await asyncio.wait_for(reader.readexactly(2), 8)
                self.assertEqual(header[0], 0x82)
                self.assertTrue(header[1] & 0x80)
                length = header[1] & 0x7f
                self.assertLess(length, 126)
                mask = await reader.readexactly(4)
                data = await reader.readexactly(length)
                data = bytes(value ^ mask[index % 4] for index, value in enumerate(data))
                if data[0] == 0x0A:
                    self.assertEqual(data[1:2], b"\0")
                    self.assertEqual(len(data), 4)
                    heartbeat.put_nowait(asyncio.get_running_loop().time() - started)
                    await self.send_frame(writer, b"\x0b\0" + data[2:4])
                    if len(connected) == 1:
                        return  # Force an actual fresh dial after first heartbeat.
                    try:
                        while await reader.read(4096):
                            pass
                    except ConnectionResetError:
                        pass  # Explicit native stop may reset instead of TLS close.
                    return

        url = await self.raw_peer(peer)
        client = self.client(url, sc_heartbeat_interval_ms=3_000,
                             sc_heartbeat_timeout_ms=3_200, reconnect=ScReconnect(
                                 initial_delay_ms=50, max_delay_ms=50, max_retries=2))
        await asyncio.wait_for(client.__aenter__(), 5)
        for _ in range(2):
            self.assertLess(await asyncio.wait_for(heartbeat.get(), 8), 8)
        self.assertEqual(connected[0], connected[1])
        await asyncio.wait_for(client.stop(), 5)
