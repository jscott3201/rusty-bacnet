"""Certificate observations for an existing configured SC client.

Installed-extension tests exercise both helpers against actual local peers.
Use show_rejected(error) in an except BacnetTransportError block around entry.
"""


async def show_accepted(client):
    peer = await client.sc_peer_info()
    if peer is None:  # disconnected or reconnecting
        return None
    print(peer.peer_address, peer.connection_id, peer.handshake_kind)
    print("leaf SHA-256:", peer.leaf_sha256.hex())
    for certificate in peer.certificates:
        metadata = certificate.metadata
        if metadata is not None:
            print(metadata.subject, metadata.issuer, metadata.not_after_unix)
    # This retained value is historical. Resumption may reuse certificates;
    # this observation is neither fresh validation nor a revocation verdict.
    return peer


def show_rejected(error):
    print(error)  # The original rejection reason is preserved.
    untrusted = error.sc_untrusted_peer
    if untrusted is not None:
        for certificate in untrusted.certificates:
            metadata = certificate.metadata
            if metadata is not None:
                print("UNTRUSTED", metadata.subject, metadata.issuer,
                      metadata.not_after_unix)
            # certificate.der explicitly exports the original untrusted bytes.
    # None means unavailable at the observation point, not proof none were sent.
    return untrusted
