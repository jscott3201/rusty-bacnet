//! Local interface address discovery for wildcard-bound UDP ingress checks.

use std::net::{Ipv4Addr, Ipv6Addr};

#[cfg(any(
    target_os = "linux",
    target_os = "l4re",
    target_os = "android",
    target_os = "emscripten",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "haiku",
    target_os = "nto",
    target_os = "hurd",
    target_os = "fuchsia",
))]
#[allow(unsafe_code)]
fn collect() -> (Vec<Ipv4Addr>, Vec<Ipv6Addr>) {
    struct IfAddrsGuard(*mut libc::ifaddrs);

    impl Drop for IfAddrsGuard {
        fn drop(&mut self) {
            // SAFETY: the pointer was returned by `getifaddrs` and this guard
            // owns the corresponding single `freeifaddrs` call.
            unsafe { libc::freeifaddrs(self.0) }
        }
    }

    let mut head = std::ptr::null_mut();
    // SAFETY: `getifaddrs` initializes `head` on success. The guarded list is
    // walked only through non-null nodes and address-family-checked pointers.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return (Vec::new(), Vec::new());
    }
    let _guard = IfAddrsGuard(head);
    let mut ipv4 = Vec::new();
    let mut ipv6 = Vec::new();
    let mut current = head;
    while !current.is_null() {
        // SAFETY: `current` is a node in the live guarded list.
        let entry = unsafe { &*current };
        if !entry.ifa_addr.is_null() {
            // SAFETY: the non-null sockaddr is valid for its reported family.
            let family = unsafe { (*entry.ifa_addr).sa_family as i32 };
            if family == libc::AF_INET {
                // SAFETY: family AF_INET selects the sockaddr_in layout.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in) };
                ipv4.push(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes()));
            } else if family == libc::AF_INET6 {
                // SAFETY: family AF_INET6 selects the sockaddr_in6 layout.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                ipv6.push(Ipv6Addr::from(address.sin6_addr.s6_addr));
            }
        }
        current = entry.ifa_next;
    }
    ipv4.sort_unstable();
    ipv4.dedup();
    ipv6.sort_unstable();
    ipv6.dedup();
    (ipv4, ipv6)
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "l4re",
    target_os = "android",
    target_os = "emscripten",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "haiku",
    target_os = "nto",
    target_os = "hurd",
    target_os = "fuchsia",
)))]
fn collect() -> (Vec<Ipv4Addr>, Vec<Ipv6Addr>) {
    (Vec::new(), Vec::new())
}

pub(crate) fn ipv4() -> Vec<Ipv4Addr> {
    collect().0
}

pub(crate) fn ipv6() -> Vec<Ipv6Addr> {
    collect().1
}

#[cfg(any(
    target_os = "linux",
    target_os = "l4re",
    target_os = "android",
    target_os = "emscripten",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "haiku",
    target_os = "nto",
    target_os = "hurd",
    target_os = "fuchsia",
))]
#[allow(unsafe_code)]
pub(crate) fn ipv6_interface_index(addr: &Ipv6Addr) -> Option<u32> {
    use std::ffi::CStr;

    struct IfAddrsGuard(*mut libc::ifaddrs);
    impl Drop for IfAddrsGuard {
        fn drop(&mut self) {
            // SAFETY: this is the matching deallocator for `getifaddrs`.
            unsafe { libc::freeifaddrs(self.0) }
        }
    }

    let mut head = std::ptr::null_mut();
    // SAFETY: the guarded list and address pointers remain valid through the walk.
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return None;
    }
    let _guard = IfAddrsGuard(head);
    let mut current = head;
    while !current.is_null() {
        // SAFETY: `current` is a node in the live guarded list.
        let entry = unsafe { &*current };
        if !entry.ifa_addr.is_null() {
            // SAFETY: the non-null sockaddr is valid for its reported family.
            let family = unsafe { (*entry.ifa_addr).sa_family as i32 };
            if family == libc::AF_INET6 {
                // SAFETY: family AF_INET6 selects the sockaddr_in6 layout.
                let address = unsafe { &*(entry.ifa_addr as *const libc::sockaddr_in6) };
                if Ipv6Addr::from(address.sin6_addr.s6_addr) == *addr {
                    // SAFETY: the OS provides a NUL-terminated interface name.
                    let index =
                        unsafe { libc::if_nametoindex(CStr::from_ptr(entry.ifa_name).as_ptr()) };
                    if index != 0 {
                        return Some(index);
                    }
                }
            }
        }
        current = entry.ifa_next;
    }
    None
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "l4re",
    target_os = "android",
    target_os = "emscripten",
    target_vendor = "apple",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "solaris",
    target_os = "illumos",
    target_os = "haiku",
    target_os = "nto",
    target_os = "hurd",
    target_os = "fuchsia",
)))]
pub(crate) fn ipv6_interface_index(addr: &Ipv6Addr) -> Option<u32> {
    let _ = addr;
    None
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(any(
        target_os = "linux",
        target_os = "l4re",
        target_os = "android",
        target_os = "emscripten",
        target_vendor = "apple",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "solaris",
        target_os = "illumos",
        target_os = "haiku",
        target_os = "nto",
        target_os = "hurd",
        target_os = "fuchsia",
    ))]
    fn local_addresses_include_loopback_on_supported_targets() {
        assert!(super::ipv4().contains(&std::net::Ipv4Addr::LOCALHOST));
        assert!(super::ipv6().contains(&std::net::Ipv6Addr::LOCALHOST));
    }
}
