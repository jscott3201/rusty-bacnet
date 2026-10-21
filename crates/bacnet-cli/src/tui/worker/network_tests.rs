//! The BDT survey reads several BBMDs at once and gives the result reading
//! them one at a time does (#1823). The fixtures run on paused time, so a
//! silent peer's three seconds cost no wall-clock time.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bvll::BvlcResultCode;
use tokio::time::Instant;

use crate::tui::network::matrix::Matrix;

use super::*;

/// The transport's wait for a BVLC response.
const TIMEOUT: Duration = Duration::from_secs(3);

fn at(n: u16) -> SocketAddrV4 {
    SocketAddrV4::new(
        Ipv4Addr::new(10, 0, (n / 200) as u8, (n % 200) as u8 + 1),
        47808,
    )
}

fn table(peers: &[u16]) -> Probe {
    Probe::Table(
        peers
            .iter()
            .map(|&n| BdtRow {
                address: at(n),
                mask: Ipv4Addr::BROADCAST,
            })
            .collect(),
    )
}

/// What each BBMD answers, and after how long; any BBMD left out is silent.
struct Fixture(HashMap<SocketAddrV4, (Duration, Probe)>);

impl Fixture {
    /// The read of `bbmd`: its probe after its delay, or no answer after
    /// the timeout.
    async fn read(&self, bbmd: SocketAddrV4) -> Probe {
        match self.0.get(&bbmd) {
            Some((delay, probe)) => {
                tokio::time::sleep(*delay).await;
                probe.clone()
            }
            None => {
                tokio::time::sleep(TIMEOUT).await;
                Probe::Unreachable("no answer".into())
            }
        }
    }
}

/// The survey as it ran before #1823: one read at a time, breadth first.
async fn one_at_a_time(fixture: &Fixture, seed: SocketAddrV4) -> Vec<(SocketAddrV4, Probe)> {
    let mut seen = HashSet::from([seed]);
    let mut pending = VecDeque::from([seed]);
    let mut probes = Vec::new();
    while let Some(bbmd) = pending.pop_front() {
        if probes.len() >= MAX_SURVEY {
            probes.push((bbmd, Probe::Skipped));
            continue;
        }
        let probe = fixture.read(bbmd).await;
        if let Probe::Table(rows) = &probe {
            for row in rows {
                if seen.insert(row.address) {
                    pending.push_back(row.address);
                }
            }
        }
        probes.push((bbmd, probe));
    }
    probes
}

/// The survey with reads at once, its time and the most reads in flight.
async fn at_once(
    fixture: &Fixture,
    seed: SocketAddrV4,
) -> (Vec<(SocketAddrV4, Probe)>, Duration, usize) {
    let (live, most) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let begin = Instant::now();
    let probes = survey_with(seed, |bbmd| {
        let (live, most) = (Rc::clone(&live), Rc::clone(&most));
        async move {
            live.set(live.get() + 1);
            most.set(most.get().max(live.get()));
            let probe = fixture.read(bbmd).await;
            live.set(live.get() - 1);
            probe
        }
    })
    .await;
    (probes, begin.elapsed(), most.get())
}

/// A seed listing three peers that never answer.
fn three_silent() -> Fixture {
    Fixture(HashMap::from([(
        at(0),
        (Duration::ZERO, table(&[0, 1, 2, 3])),
    )]))
}

#[tokio::test(start_paused = true)]
async fn three_silent_peers_cost_one_timeout_not_three() {
    let fixture = three_silent();
    let begin = Instant::now();
    let sequential = one_at_a_time(&fixture, at(0)).await;
    let sequential_took = begin.elapsed();
    let (parallel, parallel_took, most) = at_once(&fixture, at(0)).await;
    assert_eq!(parallel, sequential);
    assert_eq!(sequential_took, 3 * TIMEOUT);
    assert_eq!(parallel_took, TIMEOUT, "three timeouts should overlap");
    assert_eq!(most, 3);
}

/// A web of tables, NAKs and silent peers, larger than the survey limit,
/// whose answers arrive out of order: the grid and findings match reading
/// one at a time, and no more than four reads are ever in flight.
#[tokio::test(start_paused = true)]
async fn a_mixed_survey_matches_the_sequential_result() {
    let nak = Probe::Nak(BvlcResultCode::READ_BROADCAST_DISTRIBUTION_TABLE_NAK);
    let ms = Duration::from_millis;
    let fixture = Fixture(HashMap::from([
        (at(0), (ms(50), table(&[0, 1, 2, 3, 4, 5, 6]))),
        // The first peer answers last of its level, after a later one has
        // already listed more BBMDs.
        (at(1), (ms(900), table(&[1, 7, 8, 0]))),
        (at(2), (ms(10), nak.clone())),
        // 3 is silent.
        (
            at(4),
            (ms(20), table(&[4, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18])),
        ),
        (at(5), (ms(400), table(&[5, 4, 19]))),
        (
            at(6),
            (ms(5), Probe::Unreachable("encoding error: lost".into())),
        ),
        (at(7), (ms(30), table(&[7, 20, 21]))),
        (at(8), (ms(1), nak)),
        (at(9), (ms(250), table(&[9]))),
        (at(10), (ms(60), table(&[10, 1]))),
        // 11 to 21 are silent, or past the limit.
    ]));
    let sequential = one_at_a_time(&fixture, at(0)).await;
    let (parallel, _, most) = at_once(&fixture, at(0)).await;
    assert_eq!(parallel, sequential);
    assert_eq!(most, SURVEY_AT_ONCE);
    let read = parallel
        .iter()
        .filter(|(_, p)| *p != Probe::Skipped)
        .count();
    assert_eq!(read, MAX_SURVEY);
    assert!(parallel.len() > MAX_SURVEY, "some BBMDs go unread");
    assert_eq!(Matrix::build(&parallel), Matrix::build(&sequential));
}

#[tokio::test(start_paused = true)]
async fn a_lone_seed_is_read_once() {
    let fixture = Fixture(HashMap::from([(at(0), (Duration::ZERO, table(&[0])))]));
    let (probes, took, most) = at_once(&fixture, at(0)).await;
    assert_eq!(probes, [(at(0), table(&[0]))]);
    assert_eq!((took, most), (Duration::ZERO, 1));
}

/// Against a real BBMD on loopback, on paused time: three peers that never
/// answer cost one BVLC timeout.
#[tokio::test(start_paused = true)]
async fn a_real_survey_waits_out_silent_peers_together() {
    let mut seed = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    seed.enable_bbmd(Vec::new());
    let _seed_rx = seed.start().await.unwrap();
    let seed_at = address_of(seed.local_mac()).unwrap();
    let mut silent = Vec::new();
    for _ in 0..3 {
        silent.push(
            tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap(),
        );
    }
    let silent_at: Vec<SocketAddrV4> = silent
        .iter()
        .map(|socket| match socket.local_addr().unwrap() {
            std::net::SocketAddr::V4(at) => at,
            std::net::SocketAddr::V6(_) => unreachable!("bound to IPv4"),
        })
        .collect();
    let entry = |address: &SocketAddrV4| BdtEntry {
        ip: address.ip().octets(),
        port: address.port(),
        broadcast_mask: [255; 4],
    };
    let rows = std::iter::once(&seed_at)
        .chain(&silent_at)
        .map(entry)
        .collect();
    seed.bbmd_state()
        .unwrap()
        .lock()
        .unwrap()
        .set_bdt(rows)
        .unwrap();
    let mut client = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let _client_rx = client.start().await.unwrap();
    let begin = Instant::now();
    let probes = survey(&client, seed_at).await;
    let took = begin.elapsed();
    assert_eq!(probes.len(), 4);
    assert!(matches!(probes[0], (at, Probe::Table(_)) if at == seed_at));
    for ((at, probe), silent) in probes[1..].iter().zip(&silent_at) {
        assert_eq!(at, silent);
        assert_eq!(*probe, Probe::Unreachable("no answer".into()));
    }
    assert!(took >= TIMEOUT && took < 2 * TIMEOUT, "took {took:?}");
    client.stop().await.unwrap();
    seed.stop().await.unwrap();
}
