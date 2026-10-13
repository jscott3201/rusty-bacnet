# Security Policy

## Supported Versions

Before 1.0, security fixes land on `dev` and ship in the next release, which may
be a new minor version (for example 0.14.0). Fixes aren't backported to older
release lines, so upgrade to the latest release to receive them.

| Version                            | Supported          |
| ---------------------------------- | ------------------ |
| `dev` branch                       | :white_check_mark: |
| Latest release (currently 0.13.x)  | :white_check_mark: |
| Older releases                     | :x:                |

## Reporting a Vulnerability

If you discover a security vulnerability in rusty-bacnet, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, please email **jscott3201@gmail.com** with:

- A description of the vulnerability
- Steps to reproduce the issue
- The affected version(s)
- Any potential impact assessment

You should receive an acknowledgment within 48 hours. We will work with you to understand the issue and coordinate a fix and disclosure timeline.

## Scope

This policy covers:

- The Rust crates in this repository (`bacnet-time`, `bacnet-types`, `bacnet-encoding`, `bacnet-services`, `bacnet-mstp-codec`, `bacnet-mstp-core`, `bacnet-bip-codec`, `bacnet-bip-core`, `bacnet-router-core`, `bacnet-transport`, `bacnet-network`, `bacnet-objects`, `bacnet-client`, `bacnet-server`, `bacnet-endpoint-core`, `bacnet-endpoint`)
- The `bacnet` CLI (`bacnet-cli`)
- The Python bindings (`rusty-bacnet`)
- BACnet protocol handling (parsing, encoding, transport security)

Security issues in the BACnet/SC TLS implementation, authentication bypasses, buffer overflows in protocol decoding, and denial-of-service vectors in transport/network layers are of particular interest.
