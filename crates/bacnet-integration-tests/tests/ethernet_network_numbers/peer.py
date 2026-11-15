"""Independent raw Ethernet oracle. Run only on a task-owned isolated link."""
import json
import os
from pathlib import Path
import socket
import time

ROOT = Path(os.environ['BACNET_ETHERNET_FIXTURE_DIR'])
INTERFACE = os.environ['BACNET_ETHERNET_INTERFACE']
BROADCAST = b'\xff' * 6
OTHER = bytes.fromhex('020000000042')
MULTICAST = bytes.fromhex('01005e000042')
PEER = bytes.fromhex(Path(f'/sys/class/net/{INTERFACE}/address').read_text().strip().replace(':', ''))
QUERY = bytes([1, 128, 18])

def wait(path):
    until = time.monotonic() + 30
    while not path.exists():
        assert time.monotonic() < until, f'no progress: {path.name}'
        time.sleep(.01)

def number(value, flag=1):
    return bytes([1, 128, 19]) + value.to_bytes(2, 'big') + bytes([flag])

def frame(dst, control, payload):
    return (dst + PEER + (len(payload) + 3).to_bytes(2, 'big') + bytes([130, 130, control]) + payload).ljust(60, b'\0')

def run():
    raw = socket.socket(socket.AF_PACKET, socket.SOCK_RAW, socket.htons(3))
    raw.bind((INTERFACE, 0))
    raw.settimeout(3)
    print(json.dumps({'peer_mac': PEER.hex(), 'cap_eff': next(v for v in Path('/proc/self/status').read_text().splitlines() if v.startswith('CapEff:'))}), flush=True)
    for case in ['server-stop', 'server-drop', 'client-stop', 'client-drop']:
        wait(ROOT / f'{case}.ready')
        ready = json.loads((ROOT / f'{case}.ready').read_text())
        owner = bytes(ready['mac'])
        assert owner != PEER and len(owner) == 6 and any(owner)
        print(json.dumps(ready), flush=True)
        def send(dst, npdu, control=3):
            data = frame(dst, control, npdu)
            assert raw.send(data) == len(data)
            print(json.dumps({'case': case, 'sent': data.hex()}), flush=True)
        def expect(payload, control=3):
            until = time.monotonic() + 3
            while True:
                assert time.monotonic() < until, 'raw reply watchdog'
                data, address = raw.recvfrom(2048)
                if len(data) < 17 or data[6:12] != owner or data[14] != 130:
                    continue
                print(json.dumps({'case': case, 'received': data.hex(), 'packet_type': address[2]}), flush=True)
                expected_dst = BROADCAST if control == 3 else PEER
                assert data[:6] == expected_dst
                assert int.from_bytes(data[12:14], 'big') == 3 + len(payload)
                assert data[14:17] == bytes([130, 130 if control == 3 else 131, control])
                assert data[17:17+len(payload)] == payload
                assert len(data) == max(60, 17 + len(payload))
                assert not any(data[17+len(payload):]), 'only zero padding outside declared length'
                return
        # Same Number FIFO fences: any early reply is inspected, never discarded.
        send(owner, QUERY)
        send(BROADCAST, QUERY)
        send(BROADCAST, number(77))
        send(owner, QUERY)
        expect(number(77, 0))
        # No broadcast question once the number is known: its answer waits
        # up to ten seconds (Clause 6.4.19, #2597), tested on paused time.
        for value, flag, result in [(78, 0, 77), (79, 1, 79), (80, 0, 79), (81, 1, 81)]:
            send(BROADCAST, number(value, flag))
            send(owner, QUERY)
            expect(number(result, 0))
        for destination in [owner, OTHER, MULTICAST]:
            send(destination, number(222))
            send(owner, QUERY)
            expect(number(81, 0))
        for malformed in [number(0), number(65535), number(222, 2), number(222)[:-1], number(222)+b'\0', bytes([1,136,0,4,1,9,19,0,222,1]), bytes([1,160,255,255,0,255,19,0,222,1])]:
            send(BROADCAST, malformed)
            send(owner, QUERY)
            expect(number(81, 0))
        for value, (destination, invalid_query) in enumerate([(OTHER, QUERY), (MULTICAST, QUERY), (owner, QUERY+b'\0'), (owner, bytes([1,136,0,4,1,9,18])), (owner, bytes([1,160,255,255,0,255,18]))], 82):
            send(destination, invalid_query)
            send(BROADCAST, number(value))
            send(owner, QUERY)
            expect(number(value, 0))
        # LLC handling has its own receive-loop fence, independent of Number work.
        for destination in [owner, BROADCAST]:
            send(destination, b'x'*43, 0xaf)
            expect(bytes([129, 1, 1]), 0xbf)
            send(destination, b'T'*43, 0xe3)
            expect(b'T'*43, 0xf3)
        for destination in [OTHER, MULTICAST]:
            send(destination, b'x'*43, 0xaf)
            send(destination, b'bad'*14+b'!', 0xe3)
            send(owner, b'F'*43, 0xe3)
            expect(b'F'*43, 0xf3)
        (ROOT / f'{case}.done').touch()
        wait(ROOT / f'{case}.stopped')
        # Raw-FD count is asserted by the Rust owner. This bounded receive window
        # additionally observes silence; it alone is not the cleanup proof.
        send(owner, QUERY)
        raw.settimeout(.15)
        try:
            while True:
                data, _ = raw.recvfrom(2048)
                assert data[6:12] != owner or data[14:16] != bytes([130,130]), 'late owner output after raw-FD release'
        except TimeoutError:
            pass
        finally:
            raw.settimeout(3)
        (ROOT / f'{case}.checked').touch()
        print(json.dumps({'case': case, 'passed': True}), flush=True)
    raw.close()

try:
    run()
except BaseException as error:
    (ROOT / 'peer.error').write_text(str(error))
    raise
