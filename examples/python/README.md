# Python Examples

These examples demonstrate using `rusty_bacnet` from Python.

## Prerequisites

```bash
pip install rusty-bacnet
```

The MS/TP example requires a serial-enabled package, an RS-485 adapter that
provides automatic transmit-direction control (adapter and driver behavior
varies), and an unused local master MAC in `0..=127`. The Python transport
accepts configured baud rates of 9600, 19200, 38400, 57600, 76800, or 115200.

## Examples

| Example | Description |
|---------|-------------|
| [`bip_client_server.py`](bip_client_server.py) | BACnet/IP client and server — read, write, RPM, discovery |
| [`endpoint_bip.py`](endpoint_bip.py) | B/IP endpoint — one transport receives + initiates (migration from two connections) |
| [`frame_transcript.py`](frame_transcript.py) | Raw frames and Rust-decoded summaries while a virtual device answers and reads on one socket (development API) |
| [`mstp_mini_device.py`](mstp_mini_device.py) | BACnet MS/TP mini-device over USB RS-485 (`transport="mstp"`) |
| [`cov_subscriptions.py`](cov_subscriptions.py) | COV subscription and real-time notifications |
| [`sc_secure_connect.py`](sc_secure_connect.py) | BACnet/SC with hub, TLS, and VMAC addressing |
| [`sc_peer_certificates.py`](sc_peer_certificates.py) | Helpers for accepted-Hub snapshots and untrusted rejected-attempt certificates on configured clients |
| [`ipv6_client_server.py`](ipv6_client_server.py) | BACnet/IPv6 client and server: VMAC addressing and `resolve_vmac` |
| [`device_management.py`](device_management.py) | DeviceCommunicationControl, CreateObject, error handling |
| [`inventory.py`](inventory.py) | A device's points as records: `Device.inventory()` and the `Device.points()` async iterator |
| [`watch.py`](watch.py) | Watching points with `Device.watch()`: COV renewed before each lifetime ends, polling where COV is refused, the client's watch defaults, a watch that follows its device by instance, and `add`/`remove` on a live watch |

## Running

```bash
# BIP example (works immediately)
python bip_client_server.py

# Endpoint example (one transport, both roles)
python endpoint_bip.py

# Same-socket trap transcript; requires a build with the frame-observation API
python frame_transcript.py --interface 127.0.0.1 --port 0

# Standalone MS/TP mini-device (serial-enabled package and USB RS-485 adapter required)
python mstp_mini_device.py --serial /dev/serial/by-id/usb-... --mac 3

# IPv6 example
python ipv6_client_server.py

# COV example
python cov_subscriptions.py

# SC example (requires TLS certs — see comments in file)
python sc_secure_connect.py

# Device management
python device_management.py

# Point inventory (works immediately)
python inventory.py

# Watching points (works immediately)
python watch.py
```
