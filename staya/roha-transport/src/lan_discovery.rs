//! Local discovery is untrusted address information, not pairing.
use std::{collections::BTreeSet, net::{IpAddr, SocketAddr}, str::FromStr, time::{Duration, Instant}};
use iroh::EndpointId;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

pub const SERVICE: &str = "_akula-staya._udp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanCandidate {
    pub id: EndpointId,
    pub address: SocketAddr,
}

/// Only explicitly allowed peers can become connection candidates.
/// mDNS records are unauthenticated; QUIC still authenticates the key.
pub fn candidate(id: &str, port: u16, ip: IpAddr, allowed: &BTreeSet<EndpointId>) -> Option<LanCandidate> {
    if port == 0 || ip.is_unspecified() || ip.is_multicast() || ip.is_loopback() {
        return None;
    }
    let id = EndpointId::from_str(id).ok()?;
    allowed.contains(&id).then_some(LanCandidate {
        id,
        address: SocketAddr::new(ip, port),
    })
}

/// Advertise only for the lifetime of the explicit lab command.
/// Identifiers are visible on the LAN; do not use on untrusted WiFi.
pub fn advertise(id: EndpointId, address: SocketAddr) -> Result<ServiceDaemon, String> {
    if address.port() == 0 || address.ip().is_unspecified() || address.ip().is_loopback() {
        return Err("advertise requires a concrete LAN address and port".into());
    }
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let name = id.to_string();
    let hostname = format!("staya-{}.local.", &name[..12]);
    let service = ServiceInfo::new(
        SERVICE, &name, &hostname, address.ip(), address.port(), &[("version", "0")][..]
    ).map_err(|e| e.to_string())?;
    daemon.register(service).map_err(|e| e.to_string())?;
    Ok(daemon)
}

/// Bounded discovery. Does not enroll unknown peers or request any files.
pub fn scan(allowed: &BTreeSet<EndpointId>, duration: Duration) -> Result<Vec<LanCandidate>, String> {
    if duration.is_zero() || duration > Duration::from_secs(20) {
        return Err("discovery window must be 1..20 seconds".into());
    }
    let daemon = ServiceDaemon::new().map_err(|e| e.to_string())?;
    let events = daemon.browse(SERVICE).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + duration;
    let mut result = BTreeSet::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        if remaining.is_zero() { break; }
        match events.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                if info.get_property_val_str("version") != Some("0") { continue; }
                let Some((text_id, _)) = info.get_fullname().split_once('.') else { continue; };
                for ip in info.get_addresses_v4() {
                    if let Some(c) = candidate(text_id, info.get_port(), IpAddr::V4(ip), allowed) {
                        result.insert((c.id, c.address));
                    }
                }
            }
            Err(mdns_sd::RecvTimeoutError::Timeout) => break,
            Err(mdns_sd::RecvTimeoutError::Disconnected) => break,
            _ => {}
        }
    }
    let _ = daemon.shutdown();
    Ok(result.into_iter().map(|(id, address)| LanCandidate { id, address }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_allowed_valid_lan_addresses() {
        let known = iroh::SecretKey::generate().public();
        let stranger = iroh::SecretKey::generate().public();
        let allow = BTreeSet::from([known]);
        let id = known.to_string();
        assert!(candidate(&id, 42424, "192.168.1.3".parse().unwrap(), &allow).is_some());
        assert!(candidate(&stranger.to_string(), 42424, "192.168.1.3".parse().unwrap(), &allow).is_none());
        assert!(candidate(&id, 0, "192.168.1.3".parse().unwrap(), &allow).is_none());
        assert!(candidate(&id, 42424, "127.0.0.1".parse().unwrap(), &allow).is_none());
        assert!(candidate(&id, 42424, "0.0.0.0".parse().unwrap(), &allow).is_none());
        assert!(candidate("bad", 42424, "192.168.1.3".parse().unwrap(), &allow).is_none());
    }
}
