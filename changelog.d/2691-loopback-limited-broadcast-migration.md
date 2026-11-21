---
section: Migration notes
---
- **B/IP on loopback (Python API, CLI, Rust API, #2691):** With `interface` 127.0.0.1 and the default broadcast address, a local Who-Is (`bacnet -i 127.0.0.1 discover`, `who_is(scope="local")`) used to reach LAN devices by accident and now fails. Pass the loopback subnet's broadcast address: `broadcast_address="127.255.255.255"` in Python, `-b 127.255.255.255` in the CLI, or the `broadcast_address` argument of `BipTransport::new`. See [macOS and Windows](docs/python-api.md#loopback-interfaces-and-broadcasts).
