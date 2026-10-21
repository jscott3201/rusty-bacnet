//! TCP dial that races a host's addresses, RFC 8305 ("Happy Eyeballs") style.
//!
//! A host name can resolve to several addresses, and the first may not answer:
//! `localhost` resolves to `::1` first on Windows, and a refused loopback
//! connect takes about two seconds there, so an IPv4-only peer cost every dial
//! that long when the addresses were tried one after another (#950). Here the
//! families alternate, and each attempt gets a head start of
//! [`CONNECTION_ATTEMPT_DELAY`] before the next one begins.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::net::TcpStream;
use tokio::time::Instant;

/// How long an attempt runs alone before the next address is tried too: the
/// value RFC 8305 recommends.
const CONNECTION_ATTEMPT_DELAY: Duration = Duration::from_millis(250);

/// Resolve `host_port` ("host:port", with an IPv6 literal in brackets) and
/// order its addresses by [`interleave`] for [`connect_resolved`]. The two
/// steps are apart so a caller can tell a failed lookup from a failed dial;
/// a name that resolves to no address is a lookup failure.
pub(crate) async fn resolve(host_port: &str) -> io::Result<Vec<SocketAddr>> {
    let addrs = interleave(tokio::net::lookup_host(host_port).await?);
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{host_port} resolved to no address"),
        ));
    }
    Ok(addrs)
}

/// Race the addresses [`resolve`] returned with [`race`]. The only
/// attempt's error is returned as is when there is one address.
pub(crate) async fn connect_resolved(addrs: &[SocketAddr]) -> io::Result<TcpStream> {
    race(addrs, CONNECTION_ATTEMPT_DELAY, |addr| {
        TcpStream::connect(addr)
    })
    .await
}

/// Alternate the address families, starting with the family of the first
/// address, and drop repeats; each family keeps its resolver order.
fn interleave(addrs: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let mut first_family = Vec::new();
    let mut other_family = Vec::new();
    let mut first_is_v6 = None;
    for addr in addrs {
        if first_family.contains(&addr) || other_family.contains(&addr) {
            continue;
        }
        if addr.is_ipv6() == *first_is_v6.get_or_insert(addr.is_ipv6()) {
            first_family.push(addr);
        } else {
            other_family.push(addr);
        }
    }
    let mut ordered = Vec::with_capacity(first_family.len() + other_family.len());
    let mut first_family = first_family.into_iter();
    let mut other_family = other_family.into_iter();
    loop {
        match (first_family.next(), other_family.next()) {
            (None, None) => return ordered,
            (first, other) => ordered.extend(first.into_iter().chain(other)),
        }
    }
}

/// Try `addrs` in order. The next attempt starts whenever an attempt fails,
/// or `delay` after the latest one started, whichever comes first. The first
/// connection wins, and dropping the rest aborts them.
///
/// When every attempt fails, a single attempt's error is returned unchanged.
/// Otherwise the error names every address with its error, and its kind is
/// the most telling one any attempt got: `ConnectionRefused` (a host
/// answered), then `TimedOut` (the path was live but nothing answered in time),
/// then the first failure's kind. So a quick "network unreachable" on one
/// family doesn't hide a refusal on the other.
async fn race<T, F, Fut>(addrs: &[SocketAddr], delay: Duration, connect: F) -> io::Result<T>
where
    F: Fn(SocketAddr) -> Fut,
    Fut: Future<Output = io::Result<T>>,
{
    let start = |addr: SocketAddr| {
        let attempt = connect(addr);
        async move { (addr, attempt.await) }
    };
    let mut waiting = addrs.iter().copied();
    let mut attempts = FuturesUnordered::new();
    let mut failures = Vec::new();
    let Some(first) = waiting.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "could not resolve to any address",
        ));
    };
    attempts.push(start(first));
    let mut next_start = Instant::now() + delay;
    loop {
        let more = waiting.len() > 0;
        tokio::select! {
            biased;
            Some((addr, result)) = attempts.next() => match result {
                Ok(connection) => return Ok(connection),
                Err(error) => {
                    failures.push((addr, error));
                    if let Some(addr) = waiting.next() {
                        attempts.push(start(addr));
                        next_start = Instant::now() + delay;
                    } else if attempts.is_empty() {
                        return Err(all_failed(failures));
                    }
                }
            },
            () = tokio::time::sleep_until(next_start), if more => {
                if let Some(addr) = waiting.next() {
                    attempts.push(start(addr));
                }
                next_start = Instant::now() + delay;
            },
            else => return Err(all_failed(failures)),
        }
    }
}

fn all_failed(mut failures: Vec<(SocketAddr, io::Error)>) -> io::Error {
    if failures.len() == 1 {
        return failures.remove(0).1;
    }
    let kinds = || failures.iter().map(|(_, error)| error.kind());
    let kind = [io::ErrorKind::ConnectionRefused, io::ErrorKind::TimedOut]
        .into_iter()
        .find(|telling| kinds().any(|kind| kind == *telling))
        .or_else(|| kinds().next())
        .unwrap_or(io::ErrorKind::InvalidInput);
    let detail = failures
        .iter()
        .map(|(addr, error)| format!("{addr}: {error}"))
        .collect::<Vec<_>>()
        .join("; ");
    io::Error::new(kind, format!("every address failed ({detail})"))
}

#[cfg(test)]
#[path = "tcp_connect_tests.rs"]
mod tests;
