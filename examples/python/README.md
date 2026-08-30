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
| [`mstp_mini_device.py`](mstp_mini_device.py) | BACnet MS/TP mini-device over USB RS-485 (`transport="mstp"`) |
| [`cov_subscriptions.py`](cov_subscriptions.py) | COV subscription and real-time notifications |
| [`sc_secure_connect.py`](sc_secure_connect.py) | BACnet/SC with hub, TLS, and VMAC addressing |
| [`ipv6_client_server.py`](ipv6_client_server.py) | BACnet/IPv6 client and server |
| [`device_management.py`](device_management.py) | DeviceCommunicationControl, CreateObject, error handling |

## Running

```bash
# BIP example (works immediately)
python bip_client_server.py

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
```
