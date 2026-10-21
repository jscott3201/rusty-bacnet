# Inspect local BACnet/SC credentials

Rust's `bacnet_transport::sc_tls::inspection::{inspect_sc_pem, inspect_sc_pem_files}`
and Python's corresponding functions inspect explicitly supplied local materials
synchronously, without a hub, listener, client or event loop. This advisory report
does not change connection startup or TLS acceptance.

```python
from pathlib import Path
from rusty_bacnet import inspect_sc_pem_files

report = inspect_sc_pem_files(
    Path("node.pem"), peer_trust_path=Path("site-ca.pem"),
    private_key_path=Path("node.key"), inspected_at_unix=1792108800,
)
for cert in report.certificates:
    print(cert.role, cert.index, cert.sha256.hex(), cert.validity, cert.expiry)
    print(cert.san.values, cert.key_usage.values, cert.extended_key_usage.values)
    print(tuple((finding.area, finding.kind) for finding in cert.findings))
```

The bytes entry point takes `identity_pem`, optional `peer_trust_pem` and optional
`private_key_pem`; Python accepts `bytes` or `bytearray`. File arguments are
`str | os.PathLike[str]`, never guessed from PEM content. The identity input is
required. Omitting peer trust or the key means that input is not assessed;
supplying an empty value raises an input error. Certificates retain their original
order within each role. Identity entries precede peer-trust entries in the report.
The first identity certificate is the leaf for key comparison; there is no chain
sorting or path construction. `direct_issuer` says whether one
peer-trust certificate signed that leaf directly (its subject is the leaf's
issuer name and the leaf's signature verifies with its key): `direct`, with
`direct_issuer_index`, `not_direct`, `unknown`, or `not_assessed` without peer
trust. Python reports carry both as frozen getters with the same names.

Each entry contains SHA-256 of the exact decoded DER and optional bounded metadata.
Malformed DER produces a `certificate/malformed` finding, unavailable validity and
unassessed extension displays. No original DER or credential PEM is retained.
Python reports and their nested values are frozen, with tuples for collections;
they survive source-buffer mutation and file deletion. Debug/repr omit certificate
text and key material. Explicit metadata and extension getters return escaped text.

`inspected_at_unix` is one signed Unix-second time. Omission samples the clock once;
Python rejects bool and non-integers, and raises `OverflowError` outside signed
64-bit range. The stated validity interval includes both endpoints. Status is
`invalid_interval`, `not_yet_valid`, `expired`, `within_interval`, or `unavailable`.
Expiry is `alarm` at at most 604800 remaining seconds and `warning` at at most
2592000 seconds, only while within the interval; otherwise it is `none` or
`not_assessed`. These are date observations, not validation or revocation checks.

SAN, key usage and extended key usage each expose `assessed`, `present` and
`values`. When assessed, `present=False` establishes absence. Parse failures and
work limits can leave `assessed=False`; absence must not be inferred then. Findings
separately identify `malformed`, `duplicate`, `unsupported` and `truncated` values.
Duplicate extensions are not selected arbitrarily. SAN displays DNS, IPv4/IPv6,
email, URI, directory-name and registered-ID forms; other forms produce unsupported
findings. DNS syntax checks do not match any intended peer name. Client/server EKU
purposes are separate values; an EKU flag does not prove every usage restriction is
satisfied. Extensions other than SAN/KU/EKU are not interpreted.

`key_consistency` is `match` only after an explicit SPKI comparison through the
fixed AWS-LC Rustls provider. Other outcomes are `mismatch`, `unusable`, `unknown`
and `not_assessed`. Recognized unencrypted PKCS#1, SEC1 and PKCS#8 keys use the
first-key semantics of existing loaders; the tail after the first recognized key
is not inspected. Encrypted-key decryption is unsupported. A malformed identity
leaf can make comparison unknown. None of these outcomes, nor a `direct`
issuer, establishes that remote peers trust that issuer, remote acceptance or
BACnet/SC admission.

File inspection opens and bounded-reads identity, optional trust and optional key
once each. Unix group/world readability comes from the opened key file's mode bits;
this is advisory and excludes ACLs. Bytes input, omitted keys and other platforms
return `key_file_permissions=None`. No permissions are changed. The report describes
the bytes read at inspection time; later startup may read changed files.

Offline limits are 1 MiB total input across roles, 64 certificates and 128 KiB per
decoded certificate. Each certificate permits 64 extensions and 16 KiB of selected
extension content. Each SAN/KU/EKU display retains at most 16 values of at most 256
ASCII-escaped bytes; the four metadata text fields are each limited to 1024 bytes.
Thus retained certificate display text is at most 16 KiB per entry and 1 MiB per
report, with separately bounded fixed fields and findings. Truncation is explicit.
Bounds do not apply to existing connection adapters. Oversized inputs, PEM framing
errors and missing required PEM items stop inspection with a typed Rust input
error / Python `ValueError`; errors contain safe role/index/category information.
File errors preserve native I/O errors / Python `OSError` errno. Once PEM decoding
succeeds, certificate and key defects belong in the report.

Run [the Python example](../examples/python/sc_inspect_pem.py) against existing
credential files, or the packaged [Rust example](../crates/bacnet-transport/examples/sc_inspect_pem.rs)
with `cargo run -p bacnet-transport --features sc-tls --example sc_inspect_pem -- node.pem site-ca.pem node.key`.
The development CLI exposes these observations through
[`bacnet sc-check`](sc-check.md), adding documented local diagnostic severities
and exit statuses, and runs the same checks before every SC dial (see
[Before a dial](sc-check.md#before-a-dial)).
