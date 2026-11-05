use lan_share_core::DEFAULT_SIGNALING_PORT;
use std::net::{IpAddr, Ipv4Addr};

pub fn list_lan_ipv4() -> Vec<String> {
    let mut addrs = Vec::new();

    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if let IpAddr::V4(v4) = iface.ip() {
                if v4.is_loopback() || v4.is_unspecified() {
                    continue;
                }
                addrs.push(v4.to_string());
            }
        }
    }

    if addrs.is_empty() {
        addrs.push(Ipv4Addr::LOCALHOST.to_string());
    }
    addrs.sort();
    addrs.dedup();
    addrs
}

pub fn signaling_bind_addr(port: u16) -> String {
    format!("0.0.0.0:{}", port.max(1))
}

pub fn default_port() -> u16 {
    DEFAULT_SIGNALING_PORT
}
