"""Owned local peers for installed-native client reconnect and close tests."""
import asyncio
import base64
from dataclasses import dataclass
import hashlib
import ssl
from urllib.parse import urlsplit

import test_sc_hub_mtls as mtls
from rusty_bacnet import BACnetClient, BACnetServer


@dataclass
class RelayConnection:
    reader: asyncio.StreamReader
    writer: asyncio.StreamWriter
    accepted_at: float
    gate: asyncio.Event
    upstream: asyncio.StreamWriter | None = None


class TcpRelay:
    """Stable address forwarding opaque TLS, with explicitly owned sockets/tasks."""
    def __init__(self, url):
        self.target = urlsplit(url)
        self.gate = asyncio.Event()
        self.gate.set()
        self.accepted = asyncio.Queue()
        self.connections = []
        self.tasks = set()
        self.errors = []

    async def start(self):
        self.listener = await asyncio.start_server(self._serve, "127.0.0.1", 0)
        self.url = f"wss://localhost:{self.listener.sockets[0].getsockname()[1]}"
        return self

    def pause(self):
        self.gate = asyncio.Event()

    def resume(self, url=None):
        if url is not None:
            self.target = urlsplit(url)
        self.gate.set()

    async def next_connection(self):
        return await asyncio.wait_for(self.accepted.get(), 5)

    async def disconnect(self):
        for connection in self.connections:
            connection.writer.transport.abort()
            if connection.upstream is not None:
                connection.upstream.transport.abort()

    async def close(self):
        self.listener.close()
        await self.disconnect()
        tasks = list(self.tasks)
        for task in tasks:
            task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        await asyncio.wait_for(self.listener.wait_closed(), 3)
        if self.errors:
            raise self.errors[0]

    async def _serve(self, reader, writer):
        task = asyncio.current_task()
        self.tasks.add(task)
        connection = RelayConnection(reader, writer, asyncio.get_running_loop().time(), self.gate)
        self.connections.append(connection)
        self.accepted.put_nowait(connection)
        pumps = []
        try:
            await connection.gate.wait()
            other, connection.upstream = await asyncio.open_connection(
                self.target.hostname, self.target.port)

            async def copy(source, destination):
                while data := await source.read(65536):
                    destination.write(data)
                    await destination.drain()

            pumps = [asyncio.create_task(copy(reader, connection.upstream)),
                     asyncio.create_task(copy(other, writer))]
            done, _ = await asyncio.wait(pumps, return_when=asyncio.FIRST_COMPLETED)
            for completed in done:
                completed.result()
        except (ConnectionError, OSError):
            pass  # The tests deliberately retire or replace either TCP endpoint.
        except asyncio.CancelledError:
            raise
        except Exception as error:
            self.errors.append(error)
        finally:
            for pump in pumps:
                pump.cancel()
            await asyncio.gather(*pumps, return_exceptions=True)
            for owned in [writer, connection.upstream]:
                if owned is not None:
                    owned.close()
                    try:
                        await asyncio.wait_for(owned.wait_closed(), 3)
                    except (ConnectionError, OSError):
                        pass
            self.tasks.discard(task)


class ClientScFixture(mtls.MtlsFixture):
    """Reuse the existing certificate and independent SC framing fixtures."""
    frame = mtls.NodeIdentityMtlsTests.frame
    send_frame = mtls.NodeIdentityMtlsTests.send_frame
    read_value = mtls.NodeIdentityMtlsTests.read_value
    incumbent = mtls.NodeIdentityMtlsTests.incumbent

    def client(self, url, **overrides):
        settings = dict(transport="sc", sc_hub=url, sc_vmac=b"\x02\0\0\0\0\2",
                        sc_device_uuid=mtls.CLIENT_UUID, sc_ca_cert=self.path("site.pem"),
                        sc_client_cert=self.path("client.pem"),
                        sc_client_key=self.path("client.key"), apdu_timeout_ms=200)
        settings.update(overrides)
        client = BACnetClient(**settings)
        self.addAsyncCleanup(self.stop_server, client)
        return client

    async def serving_hub(self):
        hub = self.hub()
        self.addAsyncCleanup(self.stop_hub, hub)
        await asyncio.wait_for(hub.start(), 5)
        server = BACnetServer(
            3000, transport="sc", sc_hub=await hub.url(), sc_vmac=b"\x02\0\0\0\0\4",
            sc_device_uuid=mtls.SERVER_UUID, sc_ca_cert=self.path("site.pem"),
            sc_client_cert=self.path("server.pem"), sc_client_key=self.path("server.key"))
        self.addAsyncCleanup(self.stop_server, server)
        server.add_analog_input(0, "Reconnect AI", 64, 72.5)
        await asyncio.wait_for(server.start(), 5)
        return hub, server

    async def relay(self, url):
        relay = await TcpRelay(url).start()
        self.addAsyncCleanup(relay.close)
        return relay

    async def socket_eof(self, connection):
        """Only for a held relay: its handler has not read this stream."""
        async def drain():
            while await connection.reader.read(65536):
                pass
        await asyncio.wait_for(drain(), 3)

    async def raw_peer(self, handler):
        """A bounded independent TLS/WebSocket/SC peer, owned by this test."""
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = context.maximum_version = ssl.TLSVersion.TLSv1_3
        context.load_cert_chain(self.path("hub.pem"), self.path("hub.key"))
        context.load_verify_locations(self.path("site.pem"))
        context.verify_mode = ssl.CERT_REQUIRED
        tasks, errors = set(), []

        async def serve(reader, writer):
            task = asyncio.current_task()
            tasks.add(task)
            try:
                request = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 3)
                headers = dict(line.split(b":", 1) for line in request.split(b"\r\n")[1:]
                               if b":" in line)
                key = next(value.strip() for name, value in headers.items()
                           if name.lower() == b"sec-websocket-key")
                accept = base64.b64encode(hashlib.sha1(
                    key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest())
                writer.write(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"
                             b"Connection: Upgrade\r\nSec-WebSocket-Accept: " + accept +
                             b"\r\nSec-WebSocket-Protocol: hub.bsc.bacnet.org\r\n\r\n")
                await asyncio.wait_for(writer.drain(), 3)
                opcode, connect = await self.frame(reader, True)
                self.assertEqual(opcode, 2)
                self.assertEqual(len(connect), 30)
                self.assertEqual(connect[:4], b"\x06\0\0\1")
                await handler(reader, writer, connect)
            except asyncio.CancelledError:
                raise
            except Exception as error:
                errors.append(error)
            finally:
                # These peers model forced connection loss. Abort explicitly;
                # a graceful TLS close can wait for a stopped native reader.
                writer.transport.abort()
                try:
                    await self.close_raw(writer)
                except Exception as error:
                    errors.append(error)
                tasks.discard(task)

        listener = await asyncio.start_server(serve, "127.0.0.1", 0, ssl=context)

        async def close():
            listener.close()
            owned = list(tasks)
            for task in owned:
                task.cancel()
            await asyncio.gather(*owned, return_exceptions=True)
            await asyncio.wait_for(listener.wait_closed(), 3)
            if errors:
                raise errors[0]
        self.addAsyncCleanup(close)
        return f"wss://localhost:{listener.sockets[0].getsockname()[1]}"

    async def accept_sc(self, writer, connect):
        # Independent Annex AB.2.11 literal Connect-Accept bytes.
        await self.send_frame(writer, b"\x07\0" + connect[2:4] + mtls.HUB_VMAC +
                              mtls.HUB_UUID + b"\x05\xc4\x05\xc4")
