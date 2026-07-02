use std::net::Ipv4Addr;

fn subnet_broadcast(ip: Ipv4Addr) -> Ipv4Addr {
    let o = ip.octets();
    Ipv4Addr::new(o[0], o[1], o[2], 255)
}

pub fn default_broadcast(interface: Ipv4Addr) -> Ipv4Addr {
    if interface.is_unspecified() {
        Ipv4Addr::BROADCAST
    } else {
        subnet_broadcast(interface)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_broadcast_handles_unspecified_and_bound_interfaces() {
        assert_eq!(
            default_broadcast(Ipv4Addr::UNSPECIFIED),
            Ipv4Addr::BROADCAST
        );
        assert_eq!(
            default_broadcast(Ipv4Addr::new(192, 168, 204, 55)),
            Ipv4Addr::new(192, 168, 204, 255)
        );
    }
}
