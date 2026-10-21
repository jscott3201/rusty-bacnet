//! A port on a BBMD.

use core::net::SocketAddrV4;

use bacnet_bip_codec::{Bvll, FdtEntry};
use bacnet_time::Instant;

use super::{add, count, fan_out, Datagrams, Ingress, PortCounters, PortError};
use crate::bbmd::{Context, Rules, Tables, Target};
use crate::node::{
    Datagram, Delivery, Discard, ForBbmd, Link, Mode, Policy, Received, SendRefusal,
};

/// A port on a BBMD, which owns its [`Link`] and borrows its tables, of `B`
/// BDT rows and `F` foreign devices, with storage for the targets of one
/// forwarding decision and the rows of one Read-Foreign-Device-Table-Ack.
///
/// It applies the link's receive rules and sends its NAKs, forwards each
/// broadcast where the tables say (Annex J.4.5), and answers each
/// management request from them
/// ([`Tables::serve`](crate::bbmd::Tables::serve)) under the host's own
/// [`Rules`]. The caller only sets the BDT, restores or saves the tables,
/// and purges lapsed registrations now and then, through
/// [`tables_mut`](Self::tables_mut).
///
/// A BBMD registers nowhere and holds no broadcasts, so the port runs no
/// [`Node`](crate::node::Node), and has no timers: a BVLC-Result or table
/// ACK that reaches it answers nothing it asked, and is
/// [`Handled`](Ingress::Handled). A BBMD that also runs management
/// exchanges of its own drives a node beside the port.
///
/// The tables keep no clock of their own, so the port keeps the latest
/// time it has been given and hands them that: a `now` earlier than it is
/// taken as it, and a registration that has lapsed stays lapsed.
#[derive(Debug)]
pub struct BbmdPort<'t, const B: usize, const F: usize> {
    link: Link,
    tables: &'t mut Tables<B, F>,
    targets: &'t mut [Target],
    rows: &'t mut [FdtEntry],
    counters: PortCounters,
    /// The latest time any call has passed in.
    latest: Instant,
}

impl<'t, const B: usize, const F: usize> BbmdPort<'t, B, F> {
    /// A port on `link`, a BBMD's, with `tables`, forwarding through
    /// `targets` and reporting its foreign devices through `rows`: at
    /// least [`Tables::MAX_TARGETS`] targets, so no decision loses one, and
    /// `F` rows, so a Read-Foreign-Device-Table never fails. The link is
    /// checked as [`Node::new`](crate::node::Node::new) checks one. A link
    /// whose subnet is a multicast group serves it (Annex J.8.3): the port
    /// tells the BDT so ([`Bdt::set_serves_group`](crate::bbmd::Bdt::set_serves_group)),
    /// which refuses an own row that isn't two-hop
    /// ([`PortError::Bdt`]), and a plain link clears it.
    pub fn new(
        link: Link,
        tables: &'t mut Tables<B, F>,
        targets: &'t mut [Target],
        rows: &'t mut [FdtEntry],
    ) -> Result<Self, PortError> {
        if link.mode != Mode::Bbmd {
            return Err(PortError::Mode);
        }
        link.validate().map_err(PortError::Link)?;
        if targets.len() < Tables::<B, F>::MAX_TARGETS {
            return Err(PortError::TargetStorage {
                needed: Tables::<B, F>::MAX_TARGETS,
                available: targets.len(),
            });
        }
        if rows.len() < F {
            return Err(PortError::RowStorage {
                needed: F,
                available: rows.len(),
            });
        }
        tables
            .bdt
            .set_serves_group(link.subnet.is_group())
            .map_err(PortError::Bdt)?;
        Ok(Self {
            link,
            tables,
            targets,
            rows,
            counters: PortCounters::default(),
            latest: Instant::ZERO,
        })
    }

    /// `now`, or the latest time passed in when that is later, which it
    /// then stays (the clock rule in the crate docs).
    fn clock(&mut self, now: Instant) -> Instant {
        self.latest = self.latest.max(now);
        self.latest
    }

    /// The link.
    #[must_use]
    pub const fn link(&self) -> &Link {
        &self.link
    }

    /// The tables.
    #[must_use]
    pub fn tables(&self) -> &Tables<B, F> {
        self.tables
    }

    /// The tables, to set the BDT, restore or save them, or purge lapsed
    /// registrations.
    pub fn tables_mut(&mut self) -> &mut Tables<B, F> {
        self.tables
    }

    /// The counts since the counters were last taken, which then start
    /// again from zero: add them to your own totals, as wide as you need.
    /// Each is a `u32` that saturates between takes.
    #[must_use = "the counts are gone once taken; add them to your totals"]
    pub fn take_counters(&mut self) -> PortCounters {
        core::mem::take(&mut self.counters)
    }

    /// The counts since the counters were last taken, as
    /// [`take_counters`](Self::take_counters) would return them, without
    /// taking them.
    #[must_use]
    pub const fn counters(&self) -> PortCounters {
        self.counters
    }

    /// Take a datagram the port's socket received. The BBMD's share of it
    /// goes through the tables: a broadcast is forwarded where they say,
    /// and handed up too, and a management request is answered under
    /// `policy`'s [`Rules`]. A `now` before the latest one passed in is
    /// taken as that one.
    pub fn receive<'d>(
        &mut self,
        datagram: &Datagram<'d>,
        now: Instant,
        policy: &mut (impl Policy + Rules),
        out: &mut impl Datagrams,
    ) -> Ingress<'d> {
        let now = self.clock(now);
        let link = self.link;
        match link.receive(datagram, now, policy) {
            Received::Deliver(delivery) => Ingress::Npdu(delivery),
            Received::Nak { to, code, .. } => {
                count(&mut self.counters, out.send(to, &Bvll::BvlcResult(code)));
                Ingress::Handled
            }
            Received::Discarded(why) => Ingress::Discarded(why),
            Received::Bbmd(share) => self.share(share, &link, now, policy, out),
            // The port asks nothing, so nothing answers it.
            Received::Response(_) => Ingress::Handled,
        }
    }

    /// The BBMD's share of a message (Annex J.4.5, J.5.2).
    fn share<'d>(
        &mut self,
        share: ForBbmd<'d>,
        link: &Link,
        now: Instant,
        rules: &mut impl Rules,
        out: &mut impl Datagrams,
    ) -> Ingress<'d> {
        // The group rule was set from the link in `new`; a caller that
        // changed it through `tables_mut` broke the pairing.
        debug_assert_eq!(
            self.tables.bdt.serves_group(),
            link.subnet.is_group(),
            "the BDT's group rule no longer matches the link"
        );
        let cx = Context::new(now, F);
        let (source, npdu) = match share {
            ForBbmd::OriginalBroadcast(delivery) => {
                let sent = self.tables.original_broadcast(
                    &cx,
                    delivery.source,
                    delivery.npdu,
                    self.targets,
                );
                add(&mut self.counters, fan_out(out, sent.transmissions()));
                return Ingress::Npdu(delivery);
            }
            ForBbmd::Forwarded {
                sender,
                arrival,
                origin,
                npdu,
            } => {
                let targets = self.tables.forwarded_npdu(
                    &cx,
                    &link.subnet,
                    sender,
                    arrival,
                    origin,
                    self.targets,
                );
                let fanout = match targets {
                    Ok(fanout) => fanout,
                    Err(why) => return Ingress::NotForwarded(why),
                };
                let message = Bvll::ForwardedNpdu { origin, npdu };
                let copies = fanout
                    .targets
                    .iter()
                    .map(|target| (target.destination, message));
                add(&mut self.counters, fan_out(out, copies));
                (origin, npdu)
            }
            ForBbmd::Distribute { sender, npdu } => {
                let distribution =
                    self.tables
                        .distribute_broadcast(&cx, &link.subnet, sender, npdu, self.targets);
                match distribution {
                    Ok(sent) => add(&mut self.counters, fan_out(out, sent.transmissions())),
                    Err(why) => {
                        let nak = Bvll::BvlcResult(why.result_code());
                        count(&mut self.counters, out.send(sender, &nak));
                        return Ingress::Handled;
                    }
                }
                (sender, npdu)
            }
            ForBbmd::Management(request) => {
                // A local rule: nothing is registered, or answered, at an
                // address a reply can't reach.
                if !link.is_answerable(request.sender()) {
                    return Ingress::Discarded(Discard::UnanswerableSource);
                }
                let answer = self.tables.serve(request, now, rules, self.rows);
                count(&mut self.counters, out.send(answer.to, &answer.message));
                return Ingress::Served(answer.served);
            }
        };
        // From its origin, and to every node on the network.
        Ingress::Npdu(Delivery {
            npdu,
            source,
            group: true,
        })
    }

    /// Send `npdu` to the node at `to`, as an Original-Unicast-NPDU (Annex
    /// J.3). A group address of the link is refused before anything is
    /// encoded ([`SendRefusal::GroupDestination`], #1677).
    pub fn unicast(
        &mut self,
        to: SocketAddrV4,
        npdu: &[u8],
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        let _ = self.clock(now);
        let outgoing = self.link.unicast(to, npdu)?;
        out.send(outgoing.to, &outgoing.message)
    }

    /// Broadcast `npdu` on the subnet and, as the BBMD's own broadcast, to
    /// its peers and foreign devices (Annex J.4.5). When the socket refuses
    /// the subnet's copy, no other copy goes out. A `now` before the latest
    /// one passed in is taken as that one.
    pub fn broadcast(
        &mut self,
        npdu: &[u8],
        now: Instant,
        out: &mut impl Datagrams,
    ) -> Result<(), SendRefusal> {
        let now = self.clock(now);
        let (outgoing, _) = self.link.broadcast(npdu)?;
        out.send(outgoing.to, &outgoing.message)?;
        let cx = Context::new(now, F);
        let sent = self.tables.own_broadcast(&cx, npdu, self.targets);
        add(&mut self.counters, fan_out(out, sent.transmissions()));
        Ok(())
    }
}
