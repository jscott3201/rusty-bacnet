"""Inspect an existing site's credential files without a hub or event loop.

python examples/python/sc_inspect_pem.py node.pem --peer-trust site-ca.pem --key node.key
See docs/sc-offline-inspection.md for limits and interpretation boundaries.
"""

import argparse
from rusty_bacnet import inspect_sc_pem_files


def inspect(identity, *, peer_trust=None, key=None, inspected_at_unix=None):
    report = inspect_sc_pem_files(
        identity, peer_trust_path=peer_trust, private_key_path=key,
        inspected_at_unix=inspected_at_unix)
    print("inspected at:", report.inspected_at_unix)
    print("key consistency:", report.key_consistency)
    for cert in report.certificates:
        print(cert.role, cert.index, cert.sha256.hex(), cert.validity, cert.expiry)
        if cert.metadata is not None:
            print(cert.metadata.subject, cert.metadata.issuer)
        for finding in cert.findings:
            print("finding:", finding.area, finding.kind)
    if report.key_file_permissions is not None:
        print("key group/world read mode bits:",
              report.key_file_permissions.group_readable,
              report.key_file_permissions.world_readable)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("identity")
    parser.add_argument("--peer-trust")
    parser.add_argument("--key")
    parser.add_argument("--inspected-at-unix", type=int)
    args = parser.parse_args()
    inspect(args.identity, peer_trust=args.peer_trust, key=args.key,
            inspected_at_unix=args.inspected_at_unix)
