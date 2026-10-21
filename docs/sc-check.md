# Offline SC credential checks

`bacnet sc-check` reads local credential files and reports certificate inventory,
dates, extension observations, key consistency, whether a CA in the trust file
issued the identity leaf directly, and advisory Unix key read bits. The same
checks run before every BACnet/SC dial; see [Before a dial](#before-a-dial).
It requires the `sc-tls` feature and is available in the development checkout;
older releases may not include it. Build from the repository with:

```sh
cargo install --path crates/bacnet-cli --locked --features sc-tls
bacnet sc-check --sc-cert node.pem
bacnet --sc-cert 'credentials/node identity.pem' sc-check \
  --sc-ca site-ca.pem --sc-key node.key --format table
bacnet sc-check --sc-cert node.pem --at-unix 1792108800 --json
```

The command runs synchronously before runtime, logging and network setup. It does
not create a client, resolve a host, open a socket, prompt for input or change
credentials/permissions. Each selected input is opened and bounded-read once.
The result describes those bytes; later connection startup may read other bytes.

`--sc-cert <FILE>` is required. `--sc-ca <FILE>` and `--sc-key <FILE>` are optional;
omission is explicitly unassessed, while an empty supplied file is an input error.
Paths can appear before or after `sc-check`, including spaces, Unicode and native
OS path representations. Filesystems may reject particular native byte sequences.
Identity entries precede peer-trust entries and keep original zero-based indices.
Identity index 0 is the leaf for dates, key comparison and the direct-issuer
check. No chain is sorted and no path is built: the check asks only whether one
trust-file certificate has the leaf's issuer name as its subject and verifies
the leaf's signature with its key. Certificates after the leaf in the identity
file play no part, because Annex AB.7.4's default peer check accepts only a
leaf signed directly by a configured CA.

`--at-unix <SECONDS>` accepts the full signed 64-bit range, including negative
values. Omission samples the clock once. This is the only local command option.
`--format table` forces human output through a pipe; `--format json` forces JSON.
Otherwise the terminal gets human output and a pipe gets JSON. `--json` takes
precedence over `--format table`, but an invalid format is always a usage error.
Verbosity flags are accepted without initializing logging.

Explicit transport options are usage errors, even when their value equals the
default: `--interface`, `--port`, `--broadcast`, `--timeout`, `--min-interval-ms`,
`--foreign-bbmd`, `--ttl`, `--renewal`, `--ipv6`, `--ipv6-interface`,
`--device-instance`, `--sc`, `--sc-url`, `--sc-vmac`, and `--sc-device-uuid`.
Help remains available in builds without `sc-tls`; execution then returns 3 with
rebuild advice before opening any input.

## Reports and status

Human output includes every certificate's role/index, full SHA-256, metadata,
validity/expiry, SAN, KU, EKU and findings, followed by key observations and all
diagnostics. Dates use UTC plus exact Unix seconds, or a Unix fallback outside
the calendar formatter's range. Remaining/elapsed time includes days and a
sub-day remainder. Both formats retain escaped text and explicit truncation.

| Exit | Meaning |
| --- | --- |
| 0 | Completed without local errors; warnings and unassessed checks can remain. Also normal help/version success. |
| 1 | Completed with local errors; the complete report is still emitted. |
| 2 | Usage error: missing identity, invalid time/format or explicit transport options. Clap writes stderr; stdout has no result. |
| 3 | Input/I/O/feature execution failure, or output write/flush failure. |

An expired, future or invalid-interval identity leaf is a local error. The same
date observations on other inventory entries are warnings. Seven/thirty-day
expiry urgency is a warning and never changes exit 0 to 1 by itself. The exact
remaining seconds determine urgency: at most 604800 seconds is `alarm`, at most
2592000 is `warning`, only within the inclusive validity interval. Malformed DER
is a completed local error; malformed PEM prevents completion. A mismatched or
unusable supplied key is a local error, while an unknown comparison is a warning.

Exit 0 does not predict successful TLS or establish standards compliance. A
leaf that no trust-file CA issued directly is a warning, not an error. Whether
remote peers accept the issuer (their own CA stores), certificate-path
validation, expected peer-name matching, revocation and remote acceptance remain
unassessed. Apart from the one direct-issuer signature, the command validates no
signatures and does not enforce SAN/KU/EKU profile requirements.

## JSON fields

After valid dispatch, JSON output is exactly one newline-terminated document.
No tracing, banner or human diagnostic shares stdout. JSON execution failures
use the same top-level fields and leave stderr empty. A write/flush failure
returns 3, attempts a short stderr explanation, and may leave partial stdout;
partial output is not a report. Closed pipes follow this rule without a panic.

| Field | Type and meaning |
| --- | --- |
| `outcome` | String: `completed` or `not_completed`. Local errors can accompany completion. |
| `summary` | Object with integer `errors`, `warnings`, `notices`, counting diagnostics; null on execution failure. |
| `inspection` | Object described below; null on execution failure. |
| `diagnostics` | Array of objects with string `code`, `severity`, `message`, nullable string `role`, nullable integer `index`. Empty on execution failure. |
| `unassessed` | Array: `issuer_authorization`, `certificate_path`, `peer_name_matching`, `revocation`, `remote_acceptance`. |
| `error` | Null when completed; otherwise an execution-error object. |

Inspection fields are integer `inspected_at_unix`, boolean `peer_trust_supplied`,
string `key_consistency`, nullable object `key_file_permissions`, string
`direct_issuer`, nullable integer `direct_issuer_index`, and array
`certificates`. Key consistency is `not_assessed`, `match`, `mismatch`, `unusable`
or `unknown`. Direct issuer is `not_assessed` (no trust file), `direct`,
`not_direct` or `unknown` (the leaf is unreadable, or a candidate CA's signature
algorithm is unsupported); `direct_issuer_index` is the trust-file index of the
issuing CA when `direct`, otherwise null. Permission objects contain booleans `group_readable` and
`world_readable`; omission and non-Unix platforms yield null. These are mode bits
of the opened key file; ACLs are not assessed.

Certificate fields are string `role` (`identity` or `peer_trust`), integer `index`,
64-character lowercase hex `sha256`, nullable object `metadata`, string `validity`,
string `expiry`, three extension objects (`san`, `key_usage`,
`extended_key_usage`), and array `findings`.

Metadata fields are strings `subject`, `issuer`, `public_key_algorithm_oid`,
`signature_algorithm_oid`; signed integers `not_before_unix`, `not_after_unix`;
and boolean `text_truncated`. Timestamps are exact signed 64-bit integers. Use an
integer-capable JSON consumer; floating-point conversion may lose precision.

Validity is `unavailable`, `invalid_interval`, `not_yet_valid`, `expired` or
`within_interval`. Expiry is `not_assessed`, `none`, `warning` or `alarm`.
Extension objects contain booleans `assessed`, `present` and string array `values`.
`assessed=false` cannot establish absence. Findings contain string `area` and
`kind`: areas are `certificate`, `san`, `key_usage`, `extended_key_usage`; kinds
are `malformed`, `duplicate`, `unsupported`, `truncated`.

The error object contains string `category` (`input`, `io`, `feature_unavailable`),
nullable string `role`, nullable integer `index`, nullable string `kind`, nullable
integer `os_code`, and bounded string `message`. Input kinds are `input_too_large`,
`certificate_too_large`, `too_many_certificates`, `empty_certificates`,
`malformed_pem`, `missing_private_key`, `clock_out_of_range`. Input errors retain
role/index when available and have null `os_code`. I/O errors retain role and
native OS code when available, with null `index`/`kind`. Feature errors have
null role/index/kind/OS code. Supplied paths, source-error text and credentials
are excluded. Parse/help output is ordinary Clap output, never a JSON result.

## Diagnostic codes

Codes and enum spellings identify conditions independently of English messages.
Object-key ordering, indentation and wording are not parsing contracts. Derived
diagnostics sort by identity, peer trust, then private key; within each role they
sort by index, area and code. Certificate indices retain input order.

| Code | Severity and meaning |
| --- | --- |
| `certificate.malformed` | Error: a decoded inventory item could not be interpreted. |
| `certificate.truncated` | Warning: extension-count work bound; extension presence may be unassessed. |
| `certificate.duplicate`, `certificate.unsupported` | Warning if supplied by the native report. |
| `san.malformed`, `san.duplicate`, `san.unsupported`, `san.truncated` | Warning for that extension's interpretation limitation. |
| `key_usage.malformed`, `key_usage.duplicate`, `key_usage.unsupported`, `key_usage.truncated` | Warning for that extension's interpretation limitation. |
| `extended_key_usage.malformed`, `extended_key_usage.duplicate`, `extended_key_usage.unsupported`, `extended_key_usage.truncated` | Warning for that extension's interpretation limitation. |
| `validity.invalid_interval`, `validity.not_yet_valid`, `validity.expired` | Error for identity index 0; warning for any other certificate. |
| `validity.unavailable` | Notice: no interval assessment was possible. |
| `expiry.alarm`, `expiry.warning` | Warning: seven/thirty-day renewal urgency. |
| `metadata.text_truncated` | Warning: bounded metadata display was truncated. |
| `san.not_assessed`, `key_usage.not_assessed`, `extended_key_usage.not_assessed` | Notice: presence/values were not fully assessed. |
| `key.not_assessed` | Notice: no key was supplied. |
| `key.mismatch`, `key.unusable` | Error: supplied key mismatch or unusable representation. |
| `key.unknown` | Warning: comparison did not establish a match. |
| `issuer.not_direct` | Warning: no trust-file CA signed the identity leaf directly. |
| `issuer.unknown` | Warning: direct issuance could not be established. |
| `key_permissions.not_assessed` | Notice: Unix read bits unavailable or no key supplied. |
| `key_permissions.group_readable`, `key_permissions.world_readable` | Warning: the corresponding mode bit was observed. |
| `peer_trust.not_assessed` | Notice: no peer-trust file supplied. |

Absent SAN/KU/EKU and matching keys add no diagnostic. Metadata truncation is
separate from extension findings. All codes in the table are projected by the
command; some native enum combinations are reserved rather than currently
produced by the parser.

Complete illustrative documents are linked for
[success](sc-check-examples/success.json),
[five-day warning](sc-check-examples/warning.json),
[completed local error](sc-check-examples/local-error.json), and
[execution error](sc-check-examples/execution-error.json). They were produced
with an ephemeral example identity/key and explicit times; the certificate
fingerprint is illustrative, not an operational credential.

## Bounds

The existing inspection API bounds selected input to 1 MiB total, 64 certificates
and 128 KiB per decoded certificate. Each certificate permits 64 extensions and
16 KiB of selected extension data. Each extension display keeps at most 16 values,
each at most 256 escaped ASCII bytes; four metadata strings each permit 1024
bytes. Findings preserve truncation or unsupported interpretation explicitly.
JSON structure and escaping add bytes, so serialized output is not limited to
1 MiB. PEM, DER and private key contents are never output.

For API behavior and qualification limits, see the
[shared inspection guide](sc-offline-inspection.md). Consumer tests exercise
literal statuses and complete JSON, human output, bounded errors, native paths,
extension limits, hostile text, deterministic ordering and output failures.

## Before a dial

Every BACnet/SC client build runs these checks on its `--sc-cert`, `--sc-ca` and
`--sc-key` files (or a profile's) after the presence checks and before any DNS
lookup or TCP connection. One-shot commands and the TUI both go through
it.

The lint never refuses what the TLS configuration would accept, so a few of
this command's errors weigh less before a dial than in its own report, which
keeps them as errors for auditing.

- **Hard failures block the dial**, with the reason on stderr (or on the TUI's
  connection preview and SC screen): an identity leaf that is expired, has its
  dates reversed, cannot be read, or starts more than five minutes after this
  computer's clock; a key that does not match the leaf or cannot be used; and
  an input that cannot be read or decoded. There is no override: a hub would
  refuse those credentials anyway, later and with a vaguer error.
- **Warnings** are this command's warnings, plus those reweighed errors:
  expiry within 30 days (`expiry.warning`) or 7 days (`expiry.alarm`), a leaf
  no trust-file CA issued directly (`issuer.not_direct`), a key file others can
  read, interpretation limits, a leaf that starts at most five minutes ahead
  (likely clock skew), an unreadable CA-file or chain entry (the TLS
  configuration decides about those when it loads them), and files past the
  inspection limits above (`credentials.too_many_certificates` and the like:
  not assessed, still checked by TLS).
  - In the TUI, a profile with warnings connects only after `y` on the preview
    accepts the warnings shown. The check runs again just before the dial; a
    warning that was not on the preview (a file changed meanwhile) refuses the
    connection until it is reviewed. The client build reads the files once
    more; a warning that first appears at that point is only logged.
  - One-shot commands log each warning on stderr and dial. With
    `--format json` or `--json` these are log lines on stderr beside the JSON
    result on stdout, never part of it. A non-interactive script should not
    start failing a month before a renewal, so there is no flag: run
    `bacnet sc-check` in monitoring when the warnings themselves matter, and
    act on its exit status or JSON.
- Notices (what was not assessed) are not shown before a dial.

The direct-issuer check verifies the leaf's signature with any algorithm the
signature library knows, SHA-1 included. A SHA-1-signed leaf can therefore
report `direct` here although a WebPKI-based peer, this stack's hub among
them, refuses SHA-1 signatures.

The TUI's SC screen (`5`, then Tab) has a **My credentials** view with the
inventory the last dial used: each certificate's subject, issuer, key
algorithm, SAN and EKU, the days left in its validity window, the key match,
the direct-issuer result and every finding. A refused dial opens that view.

## Hub name verification

Annex AB.7.4 describes the default check of a peer's operational certificate:
well formed, within its validity dates, not revoked when that is known, and
signed directly by a configured CA. It leaves name checks (Common Name,
Distinguished Name or SAN matching) to installations that turn them on.

This node's hub dial checks more than that, and the extra checks are kept on
purpose:

- The hub's certificate must name the host in the hub URL: a DNS SAN for a
  host name, an IP SAN for an address. The subject's Common Name is not used.
- If the hub's certificate lists extended key usages, server authentication
  must be one of them.
- The hub may send an intermediate certificate, and a chain through it to a
  configured CA is accepted.

A hub certificate issued without the URL's host in its SAN is refused with a
"hub certificate is for another host" diagnosis, even where a peer using only
the default check would accept it. Reissue the hub's certificate with that name
or address, or dial the name it already carries. Accepting any name would let
any certificate from the site CA impersonate the hub, so there is no switch to
turn the check off. The direct-issuer check above concerns this node's own
leaf, which peers using the default check verify.

## A lab PKI with OpenSSL

There is no PKI generator in the CLI. For a lab, OpenSSL 1.1.1 or later makes a
site CA and certificates signed directly by it, with EC P-256 keys that this
stack supports:

```sh
# The site CA, self-signed for ten years.
openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout site-ca.key -out site-ca.pem -days 3650 -subj "/CN=Lab site CA" \
  -addext "basicConstraints=critical,CA:TRUE" \
  -addext "keyUsage=critical,keyCertSign,cRLSign"

# A node: a key and a request, then a certificate signed by the site CA.
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout node.key -out node.csr -subj "/CN=lab-node-1"
printf 'keyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth,serverAuth\n' > node.ext
openssl x509 -req -in node.csr -CA site-ca.pem -CAkey site-ca.key \
  -CAcreateserial -days 365 -extfile node.ext -out node.pem

# A hub: the same, naming the host or address in its URL.
openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
  -keyout hub.key -out hub.csr -subj "/CN=lab-hub"
printf 'keyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth,clientAuth\nsubjectAltName=DNS:hub.lab.example,IP:192.0.2.10\n' > hub.ext
openssl x509 -req -in hub.csr -CA site-ca.pem -CAkey site-ca.key \
  -CAcreateserial -days 365 -extfile hub.ext -out hub.pem

chmod 600 site-ca.key node.key hub.key
bacnet sc-check --sc-ca site-ca.pem --sc-cert node.pem --sc-key node.key
```

Keep `site-ca.key` off the nodes. Give every node and hub `site-ca.pem` as its
CA file, so each side's default check finds the other's issuer directly.
