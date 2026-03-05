# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

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

- The Rust crate workspace (`bacnet-types`, `bacnet-encoding`, `bacnet-services`, `bacnet-transport`, `bacnet-network`, `bacnet-objects`, `bacnet-client`, `bacnet-server`)
- The Python bindings (`rusty-bacnet`)
- BACnet protocol handling (parsing, encoding, transport security)

Security issues in the BACnet/SC TLS implementation, authentication bypasses, buffer overflows in protocol decoding, and denial-of-service vectors in transport/network layers are of particular interest.
