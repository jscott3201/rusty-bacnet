//! Owner-local emitters for hub malformed-frame diagnostics.
//!
//! Each emitter bounds one per-frame `warn!`/`debug!` to at most one event per
//! second per connection via the shared [`DiagnosticThrottle`], counting the
//! rest as suppressed for the next summary. NAK/relay/silence decisions stay
//! with the caller bit-for-bit; these helpers only gate log emission.

use std::net::SocketAddr;

use tracing::{debug, warn};

use crate::sc::diagnostic_throttle::DiagnosticThrottle;
use crate::sc_frame::{ScFunction, Vmac};

use super::HUB_MAX_BVLC_LENGTH;

/// Generic throttled emission: logs once per window, counts the rest.
pub(super) fn emit(diag: &mut DiagnosticThrottle, log: impl FnOnce(u64)) {
    if diag.should_emit_now() {
        let suppressed = diag.take_suppressed();
        log(suppressed);
    }
}

pub(super) fn oversize(diag: &mut DiagnosticThrottle, peer_addr: SocketAddr, len: usize) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: frame from {peer_addr} is {len} bytes, exceeds hub Max-BVLC-Length {HUB_MAX_BVLC_LENGTH}, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: frame from {peer_addr} is {len} bytes, exceeds hub Max-BVLC-Length {HUB_MAX_BVLC_LENGTH}, dropping");
        }
    });
}

pub(super) fn decode_error(
    diag: &mut DiagnosticThrottle,
    peer_addr: SocketAddr,
    e: &bacnet_types::error::Error,
) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: decode error from {peer_addr}: {e} (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: decode error from {peer_addr}: {e}");
        }
    });
}

pub(super) fn npdu_exceeds(diag: &mut DiagnosticThrottle) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: NPDU exceeds local Max-NPDU-Length, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: NPDU exceeds local Max-NPDU-Length, dropping");
        }
    });
}

pub(super) fn result_before_connect(diag: &mut DiagnosticThrottle, peer_addr: SocketAddr) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            debug!("Hub: Result before ConnectRequest from {peer_addr}, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            debug!("Hub: Result before ConnectRequest from {peer_addr}, dropping");
        }
    });
}

pub(super) fn npdu_before_connect(diag: &mut DiagnosticThrottle, peer_addr: SocketAddr) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: EncapsulatedNpdu before ConnectRequest from {peer_addr} — sending NAK (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: EncapsulatedNpdu before ConnectRequest from {peer_addr} — sending NAK");
        }
    });
}

pub(super) fn originating_vmac(diag: &mut DiagnosticThrottle, peer_addr: SocketAddr) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: EncapsulatedNpdu from {peer_addr} had Originating VMAC, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: EncapsulatedNpdu from {peer_addr} had Originating VMAC, dropping");
        }
    });
}

pub(super) fn missing_destination(diag: &mut DiagnosticThrottle, peer_addr: SocketAddr) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: EncapsulatedNpdu from {peer_addr} missing Destination VMAC, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: EncapsulatedNpdu from {peer_addr} missing Destination VMAC, dropping");
        }
    });
}

pub(super) fn preserve_failure(diag: &mut DiagnosticThrottle, peer_addr: SocketAddr) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: failed to preserve EncapsulatedNpdu frame from {peer_addr} (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: failed to preserve EncapsulatedNpdu frame from {peer_addr}");
        }
    });
}

pub(super) fn broadcast_npdu_drop(
    diag: &mut DiagnosticThrottle,
    npdu_len: usize,
    max_npdu: u16,
    vmac: Vmac,
) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: broadcast NPDU ({npdu_len} bytes) exceeds target max_npdu ({max_npdu}) for {vmac:02x?}, dropping for target (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: broadcast NPDU ({npdu_len} bytes) exceeds target max_npdu ({max_npdu}) for {vmac:02x?}, dropping for target");
        }
    });
}

pub(super) fn broadcast_bvlc_drop(
    diag: &mut DiagnosticThrottle,
    relay_len: usize,
    max_bvlc: u16,
    vmac: Vmac,
) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: broadcast BVLC ({relay_len} bytes) exceeds target max_bvlc ({max_bvlc}) for {vmac:02x?}, dropping for target (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: broadcast BVLC ({relay_len} bytes) exceeds target max_bvlc ({max_bvlc}) for {vmac:02x?}, dropping for target");
        }
    });
}

pub(super) fn unicast_npdu_drop(
    diag: &mut DiagnosticThrottle,
    npdu_len: usize,
    max_npdu: u16,
    dest: Vmac,
) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: NPDU ({npdu_len} bytes) exceeds target max_npdu ({max_npdu}) for {dest:02x?}, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: NPDU ({npdu_len} bytes) exceeds target max_npdu ({max_npdu}) for {dest:02x?}, dropping");
        }
    });
}

pub(super) fn unicast_bvlc_drop(
    diag: &mut DiagnosticThrottle,
    relay_len: usize,
    max_bvlc: u16,
    dest: Vmac,
) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            warn!("Hub: BVLC ({relay_len} bytes) exceeds target max_bvlc ({max_bvlc}) for {dest:02x?}, dropping (suppressed {suppressed} similar diagnostics)");
        } else {
            warn!("Hub: BVLC ({relay_len} bytes) exceeds target max_bvlc ({max_bvlc}) for {dest:02x?}, dropping");
        }
    });
}

pub(super) fn no_unicast_target(diag: &mut DiagnosticThrottle, dest: Vmac) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            debug!("Hub: no client with vmac {dest:02x?} for unicast relay (suppressed {suppressed} similar diagnostics)");
        } else {
            debug!("Hub: no client with vmac {dest:02x?} for unicast relay");
        }
    });
}

pub(super) fn unknown_function(
    diag: &mut DiagnosticThrottle,
    peer_addr: SocketAddr,
    other: &ScFunction,
) {
    emit(diag, |suppressed| {
        if suppressed > 0 {
            debug!("Hub: unknown function {other:?} from {peer_addr}, sending NAK (suppressed {suppressed} similar diagnostics)");
        } else {
            debug!("Hub: unknown function {other:?} from {peer_addr}, sending NAK");
        }
    });
}
