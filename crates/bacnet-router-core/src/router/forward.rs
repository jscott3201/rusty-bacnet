//! Rewriting a frame for the next hop (Clause 6.5.4).

use super::counters::bump;
use super::host::{Event, Host};
use super::ingress::{Arrival, Discard, Received};
use super::output::{Kind, Target};
use super::Storage;
use super::MIN_PORT_LIMIT;
use crate::npdu::{Address, Npdu};
use crate::npdu::{NetworkMessageType, RejectMessageReason};
use crate::table::Route;
use crate::MAX_MAC_LEN;

/// The frame's SNET and SADR, or, when it has none, the arrival network and
/// the link source: the originator is on the arrival link.
fn source_of<'f>(at: &Arrival<'f>, npdu: &Npdu<'f>) -> Option<Address<'f>> {
    npdu.source.or(Some(Address {
        network: at.network,
        mac: at.source,
    }))
}

/// The longest reject that can arrive: both addresses at
/// [`MAX_MAC_LEN`], the hop count, the type and three octets of data. A
/// port carries at least [`MIN_PORT_LIMIT`], so its NPDU alone fits. Link data
/// attributes are outside this bound; eligibility for a later rejection
/// must also exclude the reject's message type.
pub(super) const LONGEST_REJECT: usize = 2 + 2 * (3 + MAX_MAC_LEN) + 1 + 1 + 3;
const _: () = assert!(LONGEST_REJECT < MIN_PORT_LIMIT as usize);

impl<
        const P: usize,
        const R: usize,
        const H: usize,
        const D: usize,
        const N: usize,
        G: super::GroupClassifier,
        const B: usize,
    > Storage<P, R, H, D, N, G, B>
{
    /// Drop a relay whose hop count has run out, counting it and telling
    /// the host, which logs it as the hosted router does
    /// (router/forwarding.rs:28-31, 117-120).
    fn hop_count_spent<'f>(&mut self, npdu: &Npdu<'_>, host: &mut impl Host) -> Received<'f> {
        bump(&mut self.counters.hop_count_drops);
        let network = npdu
            .destination
            .map_or(0, |destination| destination.network);
        host.observe(Event::HopCountSpent { network });
        Received::Discarded(Discard::HopCount)
    }

    /// Send `npdu` on along `route`. Onto a directly attached network it
    /// loses its DNET, DADR and hop count and goes to the DADR, or as a
    /// broadcast for DLEN 0; toward another router it keeps them, with the
    /// hop count one lower, and goes to that router. SNET and SADR are
    /// added when missing, and the rest travels unchanged.
    ///
    /// One that arrives with a hop count of 0 goes nowhere, and so does one
    /// whose route names a port the router doesn't have. Toward the next
    /// router, one that arrives with 1 goes nowhere either: the hop count
    /// comes down before the router decides (Clauses 6.2.3, 6.3.2). One too
    /// long for the egress port, as rewritten for it, draws reject reason 4
    /// naming its DNET (Clauses 6.4.4, 6.6.3.5). A reject passing through
    /// never draws one, including a later link-envelope length refusal.
    pub(super) fn forward<'f>(
        &mut self,
        at: &Arrival<'f>,
        npdu: &Npdu<'f>,
        route: &Route,
        host: &mut impl Host,
    ) -> Received<'f> {
        let last_hop = !route.is_direct() && npdu.hop_count == 1;
        if npdu.hop_count == 0 || last_hop {
            return self.hop_count_spent(npdu, host);
        }
        // A route written through `Router::table_mut` may name a port the
        // router doesn't have; the hosted router drops its traffic too.
        let Some(egress) = self.ports[..self.port_count].get(usize::from(route.port().0)) else {
            bump(&mut self.counters.unknown_port);
            return Received::Discarded(Discard::UnknownPort);
        };
        let limits = egress.max_npdu;
        let (destination, hop_count, to) = if route.is_direct() {
            let dadr = npdu
                .destination
                .map_or(&[][..], |destination| destination.mac);
            (None, 0, dadr)
        } else {
            (
                npdu.destination,
                npdu.hop_count - 1,
                route.next_hop().as_slice(),
            )
        };
        let rewritten = Npdu {
            destination,
            source: source_of(at, npdu),
            hop_count,
            ..*npdu
        };
        let limit = limits.get(to.is_empty());
        let octets = rewritten.encoded_len();
        let eligible = npdu.message_type != Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK);
        // A route is found by DNET, so retain that DNET before the rewrite.
        let dnet = npdu
            .destination
            .map_or(0, |destination| destination.network);
        if octets > limit {
            bump(&mut self.counters.egress_too_long);
            host.observe(Event::TooLong {
                port: route.port(),
                octets,
                limit,
                rejected: eligible.then_some(dnet),
            });
            if eligible {
                self.reject(
                    at,
                    npdu.source,
                    dnet,
                    RejectMessageReason::MESSAGE_TOO_LONG,
                    host,
                );
            }
            return Received::Handled;
        }
        if self.write_now(&rewritten, Kind::Forward, Target::Toward(route.port(), to)) {
            if eligible {
                self.out.current.reject_origin = self
                    .reject_origin(at, npdu.source, dnet)
                    .unwrap_or(super::RejectOrigin::NONE);
            }
            Received::Handled
        } else {
            Received::Discarded(Discard::Unwritable)
        }
    }

    /// Pass a global broadcast on to every port but the arrival one, with
    /// DNET 0xFFFF and DLEN 0 kept, SNET and SADR added when missing, and
    /// the hop count one lower; nothing when it has run out, which is when
    /// it arrives with 1 as well as 0. A port that can't carry it is left
    /// out and counted when polled, with no reject (decision D21 on #1679).
    pub(super) fn forward_global(
        &mut self,
        at: &Arrival<'_>,
        npdu: &Npdu<'_>,
        host: &mut impl Host,
    ) {
        if npdu.hop_count <= 1 {
            // Counted and reported; the frame is still delivered here.
            let _ = self.hop_count_spent(npdu, host);
            return;
        }
        let rewritten = Npdu {
            source: source_of(at, npdu),
            hop_count: npdu.hop_count - 1,
            ..*npdu
        };
        self.write_fan_out(&rewritten, Kind::Global, at.port, host);
    }
}
