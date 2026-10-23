//! Pre-TLS admission policy for *direct IP* QUIC attempts in the lab.
//! The server sees an incoming Initial first, then can ignore/refuse/retry it.
//! Rate limiting here is not a packet firewall: noq may already have parsed
//! its Initial. No remote key is trusted until the handshake completes.

use std::{collections::HashMap, net::IpAddr, time::{Duration, Instant}};

/// Aggregate IPv6 addresses into /64 groups: rotating the interface ID does not
/// mint new budgets. IPv4 remains /32; shared NAT users share the same budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    V4(std::net::Ipv4Addr),
    V6([u8; 8]),
}
impl From<IpAddr> for Source {
    fn from(ip: IpAddr) -> Self {
        match ip {
            IpAddr::V4(ip) => Self::V4(ip),
            IpAddr::V6(ip) => {
                let octets = ip.octets();
                Self::V6(octets[..8].try_into().expect("8 byte IPv6 prefix"))
            }
        }
    }
}

const GLOBAL_BURST: f64 = 512.0;
const GLOBAL_REFILL_PER_SEC: f64 = 512.0;
const SOURCE_BURST: f64 = 160.0;
const SOURCE_REFILL_PER_SEC: f64 = 128.0;
const MAX_SOURCES: usize = 512;
const STALE_AFTER: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    at: Instant,
}
impl Bucket {
    fn full(tokens: f64, now: Instant) -> Self { Self { tokens, at: now } }
    fn refill(&mut self, max: f64, per_sec: f64, now: Instant) {
        if now > self.at {
            self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * per_sec).min(max);
            self.at = now;
        }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct PreAuthMetrics {
    pub accepted_attempts: u64,
    pub dropped_over_budget: u64,
    pub dropped_new_sources: u64,
    pub issued_retries: u64,
    pub started_handshakes: u64,
    pub dropped_no_slots: u64,
    pub timed_out_handshakes: u64,
}

/// Bounded state and a separate whole-device budget before TLS is accepted.
/// This gate counts *attempts* by observed IP/prefix, not identities or bytes.
pub struct PreAuthGate {
    global: Bucket,
    sources: HashMap<Source, Bucket>,
    metrics: PreAuthMetrics,
}
impl PreAuthGate {
    pub fn new(now: Instant) -> Self {
        Self {
            global: Bucket::full(GLOBAL_BURST, now),
            sources: HashMap::new(),
            metrics: PreAuthMetrics::default(),
        }
    }

    pub fn metrics(&self) -> PreAuthMetrics { self.metrics }

    fn prune(&mut self, now: Instant) {
        self.sources.retain(|_, bucket| {
            now.checked_duration_since(bucket.at)
                .map_or(true, |age| age < STALE_AFTER)
        });
    }

    /// Called immediately after accept() returns an Incoming and before any
    /// TLS handshake has been accepted. Failure must ignore the Incoming.
    pub fn admit(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.global.refill(GLOBAL_BURST, GLOBAL_REFILL_PER_SEC, now);
        if self.global.tokens < 1.0 {
            self.metrics.dropped_over_budget += 1;
            return false;
        }
        let source = Source::from(ip);
        if !self.sources.contains_key(&source) {
            if self.sources.len() >= MAX_SOURCES {
                self.prune(now);
            }
            if self.sources.len() >= MAX_SOURCES {
                self.metrics.dropped_new_sources += 1;
                return false;
            }
            self.sources.insert(source, Bucket::full(SOURCE_BURST, now));
        }
        let bucket = self.sources.get_mut(&source).expect("inserted source");
        bucket.refill(SOURCE_BURST, SOURCE_REFILL_PER_SEC, now);
        if bucket.tokens < 1.0 {
            self.metrics.dropped_over_budget += 1;
            return false;
        }
        self.global.tokens -= 1.0;
        bucket.tokens -= 1.0;
        self.metrics.accepted_attempts += 1;
        true
    }
    pub fn retry(&mut self) { self.metrics.issued_retries += 1; }
    pub fn started(&mut self) { self.metrics.started_handshakes += 1; }
    pub fn no_slot(&mut self) { self.metrics.dropped_no_slots += 1; }
    pub fn handshake_timeout(&mut self) { self.metrics.timed_out_handshakes += 1; }

    #[cfg(test)]
    fn tracked_sources(&self) -> usize { self.sources.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floods_from_one_ip_exhaust_shared_attempt_budget() {
        let now = Instant::now();
        let mut gate = PreAuthGate::new(now);
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        for _ in 0..160 { assert!(gate.admit(ip, now)); }
        for _ in 0..100 { assert!(!gate.admit(ip, now)); }
        let m = gate.metrics();
        assert_eq!(m.accepted_attempts,160);
        assert_eq!(m.dropped_over_budget,100);
        assert!(gate.admit(ip, now + Duration::from_secs(1)));
    }

    #[test]
    fn rotated_ipv6_interface_ids_share_one_prefix_budget() {
        let now = Instant::now();
        let mut gate = PreAuthGate::new(now);
        let a: IpAddr = "2001:db8:1111:2222::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1111:2222::999".parse().unwrap();
        for _ in 0..159 { assert!(gate.admit(a, now)); }
        assert!(gate.admit(b, now));
        assert!(!gate.admit(b, now));
    }

    #[test]
    fn whole_device_budget_survives_many_distinct_ips() {
        let now = Instant::now();
        let mut gate = PreAuthGate::new(now);
        for k in 0..512 {
            let ip = IpAddr::V4(std::net::Ipv4Addr::new(10,(k/256) as u8,(k%256) as u8,1));
            assert!(gate.admit(ip, now));
        }
        assert_eq!(gate.metrics().accepted_attempts, 512);
        let extra: IpAddr = "10.20.30.40".parse().unwrap();
        assert!(!gate.admit(extra, now));
    }

    #[test]
    fn source_table_cannot_grow_without_bound() {
        let now = Instant::now();
        let mut gate = PreAuthGate::new(now);
        // Use a unique subnet for each request, allowing globally replenished budget.
        for k in 0..1000 {
            let ip=IpAddr::V4(std::net::Ipv4Addr::new(172,(k/256) as u8,(k%256) as u8,1));
            let _ = gate.admit(ip, now + Duration::from_secs(k as u64 / 100));
        }
        assert!(gate.tracked_sources() <= MAX_SOURCES);
        assert!(gate.metrics().dropped_new_sources > 0);
        // After idle expiry old source records can be removed.
        let later = now + Duration::from_secs(180);
        assert!(gate.admit("192.0.2.88".parse().unwrap(), later));
    }

    #[test]
    fn independent_ip_budgets_but_single_global_limit() {
        let now = Instant::now();
        let mut gate = PreAuthGate::new(now);
        let a="192.0.2.1".parse().unwrap();
        let b="198.51.100.2".parse().unwrap();
        for _ in 0..160 { assert!(gate.admit(a, now)); }
        assert!(!gate.admit(a, now));
        assert!(gate.admit(b, now));
    }
}
