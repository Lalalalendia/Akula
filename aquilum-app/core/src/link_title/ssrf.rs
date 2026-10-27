//! Enforce public destinations at the socket resolver, not at a DNS preflight.
//! This agent is deliberately separate from the signed updater's HTTP client.
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::Duration;
use ureq::{Agent, AgentBuilder};
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(a == 0 || a == 10 || a == 127 || a >= 224
                || a == 100 && (64..=127).contains(&b)
                || a == 169 && b == 254
                || a == 172 && (16..=31).contains(&b)
                || a == 192 && (b == 0 || b == 168 || b == 88 && c == 99)
                || a == 198 && (b == 18 || b == 19 || b == 51 && c == 100)
                || a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            // Global unicast only, excluding IETF protocol and documentation blocks.
            (segments[0] & 0xe000) == 0x2000
                && segments[0] != 0x2002
                && !(segments[0] == 0x2001 && (segments[1] == 0 || segments[1] == 0x0db8))
        }
    }
}

fn resolve_public(host_port: &str) -> io::Result<Vec<SocketAddr>> {
    let addrs: Vec<_> = host_port.to_socket_addrs()?.collect();
    if addrs.is_empty() || addrs.iter().any(|addr| !public_ip(addr.ip())) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "link title address is not public"));
    }
    Ok(addrs)
}

pub(super) fn public_title_agent() -> Agent {
    let timeout = Duration::from_secs(5);
    AgentBuilder::new()
        // A proxy bypasses the custom resolver and would reintroduce SSRF.
        .try_proxy_from_env(false)
        .resolver(|host_port: &str| resolve_public(host_port))
        // Never follow a server-supplied redirect to another authority.
        .redirects(0)
        .max_idle_connections(0)
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .user_agent(&format!("Aquilum/{} (local-first notes; auto-link-title)", env!("CARGO_PKG_VERSION")))
        .build()
}

#[cfg(test)]
mod tests {
    use super::{public_ip, public_title_agent, resolve_public};
    use std::net::IpAddr;

    #[test]
    fn rejects_local_metadata_and_other_special_ipv4_and_ipv6() {
        for source in [
            "127.0.0.1", "10.0.0.3", "172.16.0.1", "172.31.255.255",
            "192.168.1.1", "169.254.169.254", "100.64.0.3", "0.1.2.3",
            "192.0.2.1", "198.18.0.1", "198.51.100.5", "203.0.113.9",
            "224.0.0.1", "255.255.255.255", "::1", "::ffff:127.0.0.1",
            "fe80::1", "fc00::1", "2001:db8::1", "2001::1", "2002:7f00:1::",
        ] {
            let ip: IpAddr = source.parse().unwrap();
            assert!(!public_ip(ip), "allowed a non-public IP: {source}");
        }
        assert!(public_ip("8.8.8.8".parse().unwrap()));
        assert!(public_ip("1.1.1.1".parse().unwrap()));
        assert!(public_ip("2001:4860:4860::8888".parse().unwrap()));
    }

    #[test]
    fn resolver_rejects_direct_loopback_without_a_network_request() {
        assert!(resolve_public("127.0.0.1:80").is_err());
        assert!(resolve_public("[::1]:8080").is_err());
        assert!(resolve_public("169.254.169.254:80").is_err());
        assert!(resolve_public("8.8.8.8:443").is_ok());
        assert!(resolve_public("[2001:4860:4860::8888]:443").is_ok());
        let _agent = public_title_agent();
    }
}
