# Python Examples

These examples demonstrate using `rusty_bacnet` from Python.

## Prerequisites

```bash
pip install rusty-bacnet
```

## Examples

| Example | Description |
|---------|-------------|
| [`bip_client_server.py`](bip_client_server.py) | BACnet/IP client and server — read, write, RPM, discovery |
| [`cov_subscriptions.py`](cov_subscriptions.py) | COV subscription and real-time notifications |
| [`sc_secure_connect.py`](sc_secure_connect.py) | BACnet/SC with hub, TLS, and VMAC addressing |
| [`ipv6_client_server.py`](ipv6_client_server.py) | BACnet/IPv6 client and server |
| [`device_management.py`](device_management.py) | DeviceCommunicationControl, CreateObject, error handling |

## Running

```bash
# BIP example (works immediately)
python bip_client_server.py

# IPv6 example
python ipv6_client_server.py

# COV example
python cov_subscriptions.py

# SC example (requires TLS certs — see comments in file)
python sc_secure_connect.py

# Device management
python device_management.py
```
