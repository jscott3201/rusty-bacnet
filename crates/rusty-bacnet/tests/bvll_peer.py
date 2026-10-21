"""A raw B/IP peer on 127.0.0.1 for tests that need wire control.

It runs in a thread on a blocking socket with a short timeout, since
Windows' default event loop has no add_reader.
"""
import socket
import struct
import threading


def mac(address):
    """The six-octet B/IP MAC of an ``"ip:port"``."""
    ip, port = address.rsplit(":", 1)
    return socket.inet_aton(ip) + struct.pack(">H", int(port))


def bvll(function, payload):
    return struct.pack(">BBH", 0x81, function, 4 + len(payload)) + payload


class RawPeer:
    """Hands each datagram, with its source, to ``handler``, which returns
    the datagrams to send back (None or empty for silence); keeps what it
    received and where from."""

    def __init__(self, handler):
        self.handler = handler
        self.received = []
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.sock.bind(("127.0.0.1", 0))
        self.sock.settimeout(0.1)
        self.address = "127.0.0.1:%d" % self.sock.getsockname()[1]
        self.done = threading.Event()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        while not self.done.is_set():
            try:
                data, source = self.sock.recvfrom(1500)
            except socket.timeout:
                continue
            except OSError:
                return
            self.received.append((data, source))
            for reply in self.handler(data, source) or ():
                self.sock.sendto(reply, source)

    def send(self, data, to):
        self.sock.sendto(data, to)

    def close(self):
        self.done.set()
        self.thread.join(5)
        self.sock.close()
