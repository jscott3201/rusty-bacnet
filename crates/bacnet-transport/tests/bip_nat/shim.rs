//! A small loopback "internet" of NAT routers.
//!
//! Each site has one global address, a socket this shim binds on
//! 127.0.0.1, and one private address behind it, a transport's own socket.
//! A datagram that reaches site `j`'s global address from site `i`'s
//! private address leaves through site `i`'s router and arrives through
//! site `j`'s: the shim sends it to `j`'s private address from `i`'s global
//! socket, so `j` sees `i`'s global address as the source, the way source
//! and destination translation would leave it. A sender that is no site's
//! private address is dropped: every participant here sits behind a router.
//!
//! A port-forwarding site (a BBMD's) takes inbound datagrams at any time. A
//! dynamic site (a foreign device behind an ordinary home router) takes them
//! only while the mapping its last outbound datagram opened is fresh, as a
//! router's UDP mapping times out.

use std::net::{SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio::time::Instant;

/// How a site's router admits inbound datagrams.
#[derive(Debug, Clone, Copy)]
pub enum Router {
    /// Static port forwarding to the private address.
    PortForward,
    /// A mapping that each outbound datagram opens for this long.
    Dynamic(Duration),
}

#[derive(Debug)]
struct Site {
    router: Router,
    private: Option<SocketAddrV4>,
    /// Whether the next sender that is no site's private address is put
    /// behind this router ([`Internet::claim_next`]).
    claim_next: bool,
    /// When the private side last sent through the router.
    last_outbound: Option<Instant>,
}

/// What the shim did with the datagrams it saw.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Delivered to a private address.
    pub delivered: u64,
    /// Dropped at a dynamic router whose mapping had expired.
    pub expired: u64,
}

#[derive(Debug, Default)]
struct Shared {
    sites: Vec<Site>,
    counts: Counts,
}

/// The routers, their global sockets and the tasks that move datagrams.
pub struct Internet {
    sockets: Vec<Arc<UdpSocket>>,
    shared: Arc<Mutex<Shared>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Internet {
    /// One router per entry of `routers`, each with a global socket on
    /// 127.0.0.1 and nothing behind it yet.
    pub async fn new(routers: &[Router]) -> Self {
        let mut sockets = Vec::new();
        for _ in routers {
            let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            sockets.push(Arc::new(socket));
        }
        let shared = Arc::new(Mutex::new(Shared {
            sites: routers
                .iter()
                .map(|&router| Site {
                    router,
                    private: None,
                    claim_next: false,
                    last_outbound: None,
                })
                .collect(),
            counts: Counts::default(),
        }));
        let tasks = (0..sockets.len())
            .map(|site| tokio::spawn(run(site, sockets.clone(), Arc::clone(&shared))))
            .collect();
        Self {
            sockets,
            shared,
            tasks,
        }
    }

    /// Site `site`'s global address.
    pub fn global(&self, site: usize) -> SocketAddrV4 {
        match self.sockets[site].local_addr().unwrap() {
            SocketAddr::V4(address) => address,
            SocketAddr::V6(_) => unreachable!("bound on 127.0.0.1"),
        }
    }

    /// Put `private` behind site `site`'s router.
    pub fn place(&self, site: usize, private: SocketAddrV4) {
        self.shared.lock().unwrap().sites[site].private = Some(private);
    }

    /// Put the next sender that is no site's private address behind site
    /// `site`'s router: a transport bound to port 0, whose address is known
    /// only once it sends. Start one such transport at a time.
    pub fn claim_next(&self, site: usize) {
        self.shared.lock().unwrap().sites[site].claim_next = true;
    }

    /// Who sits behind site `site`'s router, once known.
    pub fn private(&self, site: usize) -> Option<SocketAddrV4> {
        self.shared.lock().unwrap().sites[site].private
    }

    /// What the shim has done so far.
    pub fn counts(&self) -> Counts {
        self.shared.lock().unwrap().counts
    }
}

impl Drop for Internet {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Move what arrives at site `to`'s global socket.
async fn run(to: usize, sockets: Vec<Arc<UdpSocket>>, shared: Arc<Mutex<Shared>>) {
    let mut buf = [0u8; 2048];
    loop {
        let Ok((len, SocketAddr::V4(source))) = sockets[to].recv_from(&mut buf).await else {
            continue;
        };
        let now = Instant::now();
        let route = {
            let mut shared = shared.lock().unwrap();
            let known = shared
                .sites
                .iter()
                .position(|site| site.private == Some(source));
            let claimed = || shared.sites.iter().position(|site| site.claim_next);
            let Some(from) = known.or_else(claimed) else {
                continue;
            };
            let site = &mut shared.sites[from];
            if site.claim_next {
                site.claim_next = false;
                site.private = Some(source);
            }
            shared.sites[from].last_outbound = Some(now);
            let site = &shared.sites[to];
            let open = match site.router {
                Router::PortForward => true,
                Router::Dynamic(timeout) => site
                    .last_outbound
                    .is_some_and(|last| now.duration_since(last) <= timeout),
            };
            match (open, site.private) {
                (true, Some(private)) => {
                    shared.counts.delivered += 1;
                    Some((from, private))
                }
                (false, _) => {
                    shared.counts.expired += 1;
                    None
                }
                (true, None) => None,
            }
        };
        if let Some((from, private)) = route {
            let _ = sockets[from].send_to(&buf[..len], private).await;
        }
    }
}
