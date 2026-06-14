use std::net::IpAddr;

pub fn local_ip() -> Option<IpAddr> {
    if_addrs::get_if_addrs()
        .ok()?
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.addr {
            if_addrs::IfAddr::V4(addr) => Some(IpAddr::V4(addr.ip)),
            _ => None,
        })
        .find(|ip| !ip.is_loopback())
}
