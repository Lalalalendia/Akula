//! AK-MSG-SIM-2000: deterministic *synthetic* building/contact fixture.
//!
//! This module is TEST-ONLY, feature-gated, has no network, disk, user data,
//! actual Bluetooth discovery, or authenticated delivery. All receipt witnesses
//! below are deliberately synthetic and never compiled into Akula production.
//!
//! A link means an assumed one-tick capacity to move one opaque message,
//! NOT a measured RF packet success or an authenticated Akula peer.

use super::{
    Descriptor, Event, OutboundMessage, Phase, ReceiptCandidate, ReceiptVerifier,
};
use crate::sync::DeviceId;
use std::cmp::Ordering;
use uuid::Uuid;

pub(super) const DEVICE_COUNT: usize = 2_000;
pub(super) const FLOOR_COUNT: usize = 5;
pub(super) const ROOMS_PER_FLOOR: usize = 70;
pub(super) const STAIR_X: [u8; 5] = [5, 11, 17, 23, 29];
const HALL_SLOTS: usize = 35;
const MINUTE: u64 = 60_000;
const SIM_BYTES: u64 = 1_024;
const MAX_COPIES: usize = 6;
const RECEIPT_WITNESS: &[u8] = b"test-only-synthetic-recipient-witness";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Links {
    SharedWifi,
    MixedWifiBle,
    BleOnly,
    ExclusiveWifiCellBle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Movement {
    Stationary,
    Quarter,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Outages {
    None,
    SingleWindow,
    Repeated,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Scenario {
    pub links: Links,
    pub movement: Movement,
    pub outages: Outages,
    pub seed: u64,
    pub ticks: u16,
    pub ttl_ticks: u16,
    pub messages: usize,
}

impl Scenario {
    pub fn baseline(links: Links, movement: Movement, outages: Outages) -> Self {
        Self {
            links,
            movement,
            outages,
            seed: 0x4134_4b55_4c41_3031,
            ticks: 84,
            ttl_ticks: 40,
            messages: 60,
        }
    }

    fn validate(self) {
        assert!(self.ticks > 2 && self.ticks < 10_000);
        assert!(self.ttl_ticks > 0 && self.ttl_ticks < self.ticks);
        assert!(self.messages > 0 && self.messages <= DEVICE_COUNT);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Zone {
    Room,
    Hall,
    Stair(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Position {
    floor: u8,
    x: u8,
    side: u8,
    zone: Zone,
}

impl Position {
    fn room(floor: u8, room: u8) -> Self {
        Self {
            floor,
            x: room / 2,
            side: room % 2,
            zone: Zone::Room,
        }
    }

    fn index(self) -> usize {
        self.floor as usize * HALL_SLOTS + self.x as usize
    }

    fn distance_to(self, other: Self) -> u16 {
        (self.floor.abs_diff(other.floor) as u16) * 48
            + self.x.abs_diff(other.x) as u16
            + u16::from(self.side != other.side)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Radios {
    wifi: bool,
    cell: bool,
    ble: bool,
}

impl Radios {
    fn has_ip(self) -> bool {
        self.wifi || self.cell
    }
}

#[derive(Clone, Debug)]
struct Device {
    id: usize,
    radios: Radios,
    position: Position,
    destination: Position,
    dwell_until: u16,
    journey: u64,
    may_forward: bool,
    moves: bool,
}

impl Device {
    fn new(id: usize, scenario: Scenario) -> Self {
        // Exactly 400 devices per floor and 5-6 per room at t=0.
        let floor = (id % FLOOR_COUNT) as u8;
        let room = ((id / FLOOR_COUNT) % ROOMS_PER_FLOOR) as u8;
        let radios = match scenario.links {
            Links::SharedWifi => Radios {
                wifi: true,
                cell: false,
                ble: false,
            },
            Links::MixedWifiBle => Radios {
                wifi: id % 3 != 0,
                cell: false,
                ble: true,
            },
            Links::BleOnly => Radios {
                wifi: false,
                cell: false,
                ble: true,
            },
            Links::ExclusiveWifiCellBle => Radios {
                wifi: id % 3 == 0,
                cell: id % 3 == 1,
                ble: id % 3 == 2,
            },
        };
        let moves = match scenario.movement {
            Movement::Stationary => false,
            Movement::Quarter => id % 4 == 0,
            Movement::All => true,
        };
        let initial = Position::room(floor, room);
        let mut result = Self {
            id,
            radios,
            position: initial,
            destination: initial,
            dwell_until: 0,
            journey: 0,
            // Synthetic opt-in, seeded independently of id % 5 (floor).
            // Otherwise an entire floor would wrongly have zero relays.
            may_forward: mix(scenario.seed ^ ((id as u64) << 7) ^ 0xC0FF_EE30) % 5 != 0,
            moves,
        };
        if moves {
            result.pick_destination(scenario.seed);
        }
        result
    }

    fn online(&self, tick: u16, outages: Outages) -> bool {
        match outages {
            Outages::None => true,
            Outages::SingleWindow => {
                !(self.id % 7 == 0 && (12..29).contains(&tick))
            }
            Outages::Repeated => {
                (usize::from(tick) + (self.id * 13) % 17) % 17 >= 5
            }
        }
    }

    fn pick_destination(&mut self, seed: u64) {
        self.journey += 1;
        let h = mix(seed ^ ((self.id as u64) << 13) ^ (self.journey << 32));
        let new_floor = if (h % 5) == 0 {
            self.position.floor
        } else {
            (self.position.floor + 1 + ((h >> 8) % 4) as u8) % 5
        };
        let room = ((h >> 16) % ROOMS_PER_FLOOR as u64) as u8;
        self.destination = Position::room(new_floor, room);
    }

    fn advance(&mut self, tick: u16, seed: u64) {
        if !self.moves || tick < self.dwell_until {
            return;
        }
        if self.position.zone == Zone::Room {
            // Every journey begins through the door; no teleport through walls.
            self.position.zone = Zone::Hall;
            return;
        }
        if self.position.floor != self.destination.floor {
            // Take a real staircase, one adjacent floor at a time.
            let stair = (0..STAIR_X.len())
                .min_by_key(|&j| self.position.x.abs_diff(STAIR_X[j]))
                .expect("five staircases");
            let stair_x = STAIR_X[stair];
            match self.position.x.cmp(&stair_x) {
                Ordering::Less => self.position.x += 1,
                Ordering::Greater => self.position.x -= 1,
                Ordering::Equal => {
                    if self.position.floor < self.destination.floor {
                        self.position.floor += 1;
                    } else {
                        self.position.floor -= 1;
                    }
                    self.position.zone = Zone::Stair(stair as u8);
                }
            }
            return;
        }
        if self.position.x != self.destination.x {
            if self.position.x < self.destination.x {
                self.position.x += 1;
            } else {
                self.position.x -= 1;
            }
            self.position.zone = Zone::Hall;
            return;
        }
        // Crossing the hallway and entering the destination room costs a tick.
        self.position.side = self.destination.side;
        self.position.zone = Zone::Room;
        self.dwell_until = tick.saturating_add(2 + ((mix(seed ^ self.id as u64 ^ self.journey) % 7) as u16));
        self.pick_destination(seed);
    }
}

// SplitMix64: deterministic, independently seeded pseudo-random event generator.
fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn contact_seed(seed: u64, tick: u16, a: usize, b: usize) -> u64 {
    let lo = a.min(b) as u64;
    let hi = a.max(b) as u64;
    mix(seed ^ (u64::from(tick) << 48) ^ (lo << 16) ^ hi)
}

fn ble_contact(a: &Device, b: &Device, tick: u16, cfg: Scenario) -> bool {
    if !a.radios.ble
        || !b.radios.ble
        || !a.online(tick, cfg.outages)
        || !b.online(tick, cfg.outages)
        || a.id == b.id
    {
        return false;
    }
    let pa = a.position;
    let pb = b.position;
    let chance = if pa.floor == pb.floor {
        match pa.x.abs_diff(pb.x) {
            0 if pa.zone == Zone::Room && pb.zone == Zone::Room
                && pa.side == pb.side => 970,
            0 if pa.zone == Zone::Room && pb.zone == Zone::Room => 330,
            0 => 850,
            1 if pa.zone != Zone::Room || pb.zone != Zone::Room => 650,
            1 => 220,
            2 if pa.zone != Zone::Room && pb.zone != Zone::Room => 300,
            _ => 0,
        }
    } else if pa.floor.abs_diff(pb.floor) == 1 {
        match (pa.zone, pb.zone) {
            (Zone::Stair(x), Zone::Stair(y)) if x == y => 680,
            _ => 0,
        }
    } else {
        0
    };
    chance > 0 && contact_seed(cfg.seed, tick, a.id, b.id) % 1000 < chance
}

fn can_send(a: &Device, b: &Device, tick: u16, cfg: Scenario) -> bool {
    if a.id == b.id
        || !a.online(tick, cfg.outages)
        || !b.online(tick, cfg.outages)
    {
        return false;
    }
    let ip = match cfg.links {
        Links::SharedWifi | Links::MixedWifiBle => a.radios.wifi && b.radios.wifi,
        Links::BleOnly => false,
        // Explicit assumption: Wi-Fi WAN and cellular IP are routable via
        // common allowed infrastructure; this is NOT NAT traversal evidence.
        Links::ExclusiveWifiCellBle => a.radios.has_ip() && b.radios.has_ip(),
    };
    ip || ble_contact(a, b, tick, cfg)
}

type Buckets = Vec<Vec<usize>>;

fn buckets(devices: &[Device]) -> Buckets {
    let mut result = vec![Vec::new(); FLOOR_COUNT * HALL_SLOTS];
    for (i, device) in devices.iter().enumerate() {
        result[device.position.index()].push(i);
    }
    result
}

fn ble_neighbors(
    device: &Device,
    devices: &[Device],
    cells: &Buckets,
    cfg: Scenario,
    tick: u16,
) -> Vec<usize> {
    if !device.radios.ble || !device.online(tick, cfg.outages) {
        return Vec::new();
    }
    let p = device.position;
    let mut out = Vec::new();
    for x in p.x.saturating_sub(2)..=(p.x + 2).min(34) {
        for &i in &cells[p.floor as usize * HALL_SLOTS + x as usize] {
            if ble_contact(device, &devices[i], tick, cfg) {
                out.push(i);
            }
        }
    }
    if let Zone::Stair(_) = p.zone {
        for floor in [p.floor.checked_sub(1), p.floor.checked_add(1)] {
            if let Some(floor) = floor.filter(|f| (*f as usize) < FLOOR_COUNT) {
                for &i in &cells[floor as usize * HALL_SLOTS + p.x as usize] {
                    if ble_contact(device, &devices[i], tick, cfg) {
                        out.push(i);
                    }
                }
            }
        }
    }
    out
}

fn best_relay(
    holders: &[usize],
    target_start: Position,
    devices: &[Device],
    cells: &Buckets,
    tick: u16,
    cfg: Scenario,
) -> Option<usize> {
    let mut candidate: Option<(u16, usize)> = None;
    for &holder in holders {
        let source = &devices[holder];
        if !source.online(tick, cfg.outages) {
            continue;
        }
        let before = source.position.distance_to(target_start);
        // BLE contacts use only real current positions (no through-wall hops).
        let mut try_candidate = |i: usize| {
            if holders.contains(&i) || !devices[i].may_forward
                || !can_send(source, &devices[i], tick, cfg)
            {
                return;
            }
            let distance = devices[i].position.distance_to(target_start);
            // Greedy forwarding uses the *original* known target room, not
            // the receiver's future position or any simulator oracle.
            if distance < before && candidate.is_none_or(|(best, _)| distance < best) {
                candidate = Some((distance, i));
            }
        };
        for i in ble_neighbors(source, devices, cells, cfg, tick) {
            try_candidate(i);
        }
        if source.radios.has_ip() {
            for i in 0..devices.len() {
                if devices[i].radios.has_ip() {
                    try_candidate(i);
                }
            }
        }
    }
    candidate.map(|(_, i)| i)
}

struct SyntheticVerifier;
impl ReceiptVerifier for SyntheticVerifier {
    fn verify(&self, candidate: &ReceiptCandidate<'_>) -> bool {
        candidate.signature == RECEIPT_WITNESS
    }
}

#[derive(Clone, Debug)]
struct SimMessage {
    id: usize,
    source: usize,
    target: usize,
    created_tick: u16,
    target_start: Position,
    holders: Vec<usize>,
    state: OutboundMessage,
    delivery_tick: Option<u16>,
    ever_relayed: bool,
}

impl SimMessage {
    fn new(id: usize, devices: &[Device], cfg: Scenario) -> Self {
        let source = (id * 37 + 7) % devices.len();
        let offset = match id % 6 {
            0 => 350,
            1 => 1,
            2 => 4,
            3 => 157,
            4 => 175,
            _ => 911,
        };
        let target = (source + offset) % devices.len();
        // Stagger sends throughout the day, including offline windows.
        // Otherwise the one-time outage would be irrelevant on shared Wi-Fi.
        let send_window = (cfg.ticks - cfg.ttl_ticks).min(40);
        let created_tick = ((id * 7) % usize::from(send_window)) as u16;
        let message_id = (id as u128 + 1).to_be_bytes();
        let desc = Descriptor {
            id: message_id,
            recipient: DeviceId::from_uuid(Uuid::from_u128(target as u128 + 1)),
            // Synthetic opaque-envelope digest; no encryption is performed.
            ciphertext_digest: *blake3::hash(&message_id).as_bytes(),
            policy_epoch: 1,
            expires_at_ms: u64::from(created_tick + cfg.ttl_ticks) * MINUTE,
        };
        let state = OutboundMessage::after_durable_enqueue(desc, u64::from(created_tick) * MINUTE)
            .expect("valid synthetic message metadata");
        Self {
            id,
            source,
            target,
            created_tick,
            target_start: devices[target].position,
            holders: vec![source],
            state,
            delivery_tick: None,
            ever_relayed: false,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Summary {
    pub messages: usize,
    pub recipient_accepted: usize,
    pub relay_only: usize,
    pub revoked: usize,
    pub expired: usize,
    pub copied_to_relays: usize,
    pub radio_or_ip_transfers: usize,
    pub assumed_bytes_transferred: u64,
    pub forbidden_cross_technology_pairs: usize,
    pub late_deliveries: usize,
    pub max_copies_observed: usize,
    pub total_delivery_ticks: u64,
    pub p50_delivery_ticks: Option<u16>,
    pub p95_delivery_ticks: Option<u16>,
}

pub(super) fn run(cfg: Scenario) -> Summary {
    cfg.validate();
    let mut devices: Vec<_> = (0..DEVICE_COUNT).map(|i| Device::new(i, cfg)).collect();
    let mut messages: Vec<_> = (0..cfg.messages)
        .map(|i| SimMessage::new(i, &devices, cfg))
        .collect();
    let mut summary = Summary {
        messages: cfg.messages,
        ..Summary::default()
    };
    if cfg.links == Links::ExclusiveWifiCellBle {
        summary.forbidden_cross_technology_pairs = messages
            .iter()
            .filter(|msg| {
                devices[msg.source].radios.ble != devices[msg.target].radios.ble
            })
            .count();
    }
    for tick in 0..cfg.ticks {
        for device in &mut devices {
            device.advance(tick, cfg.seed);
        }
        let cells = buckets(&devices);
        let now_ms = u64::from(tick) * MINUTE;
        for message in &mut messages {
            if tick < message.created_tick {
                continue;
            }
            message.state.step(now_ms, Event::Tick).expect("monotonic time");
            if message.state.phase() == Phase::Expired
                || message.state.phase() == Phase::Revoked
                || message.state.phase() == Phase::RecipientAccepted
            {
                continue;
            }
            if tick == message.created_tick + 15 && message.id % 17 == 0 {
                message
                    .state
                    .step(now_ms, Event::TrustRevoked { new_epoch: 2 })
                    .expect("new authoritative epoch");
                continue;
            }
            let arrived = message.holders.iter().any(|&holder| {
                can_send(&devices[holder], &devices[message.target], tick, cfg)
            });
            if arrived {
                message
                    .state
                    .step(now_ms, Event::RouteAvailable)
                    .expect("non-terminal");
                if message.state.phase() != Phase::InTransit {
                    message
                        .state
                        .step(now_ms, Event::SendAttempt)
                        .expect("available direct contact");
                }
                let desc = *message.state.descriptor();
                let receipt = ReceiptCandidate {
                    id: desc.id,
                    recipient: desc.recipient,
                    ciphertext_digest: desc.ciphertext_digest,
                    policy_epoch: desc.policy_epoch,
                    signature: RECEIPT_WITNESS,
                };
                // This is ONLY a synthetic oracle asserting recipient durable
                // acceptance: security tests must use real W4 verification.
                message
                    .state
                    .accept_receipt(now_ms, &receipt, &SyntheticVerifier)
                    .expect("test-only recipient witness");
                message.delivery_tick = Some(tick);
                summary.radio_or_ip_transfers += 1;
                summary.assumed_bytes_transferred += SIM_BYTES;
                continue;
            }
            if message.holders.len() < MAX_COPIES {
                if let Some(next) = best_relay(
                    &message.holders,
                    message.target_start,
                    &devices,
                    &cells,
                    tick,
                    cfg,
                ) {
                    message
                        .state
                        .step(now_ms, Event::RouteAvailable)
                        .expect("non-terminal");
                    if message.state.phase() != Phase::InTransit {
                        message
                            .state
                            .step(now_ms, Event::SendAttempt)
                            .expect("one-hop contact");
                    }
                    message.holders.push(next);
                    message.ever_relayed = true;
                    summary.copied_to_relays += 1;
                    summary.radio_or_ip_transfers += 1;
                    summary.assumed_bytes_transferred += SIM_BYTES;
                    message
                        .state
                        .step(now_ms, Event::RelayStored)
                        .expect("synthetic relay copy");
                    // The relay receipt MUST NOT count as recipient acceptance.
                    assert_ne!(message.state.phase(), Phase::RecipientAccepted);
                    continue;
                }
            }
            message
                .state
                .step(now_ms, Event::RouteLost)
                .expect("non-terminal");
        }
    }
    let end_ms = u64::from(cfg.ticks) * MINUTE;
    let mut delivery_times = Vec::new();
    for message in &mut messages {
        message.state.step(end_ms, Event::Tick).expect("monotonic expiry");
        summary.max_copies_observed = summary.max_copies_observed.max(message.holders.len());
        match message.state.phase() {
            Phase::RecipientAccepted => {
                summary.recipient_accepted += 1;
                let delivered = message.delivery_tick.expect("recipient time");
                let delay = delivered - message.created_tick;
                summary.total_delivery_ticks += u64::from(delay);
                delivery_times.push(delay);
                if delay >= 15 {
                    summary.late_deliveries += 1;
                }
            }
            Phase::Revoked => summary.revoked += 1,
            Phase::Expired => {
                summary.expired += 1;
                if message.ever_relayed {
                    summary.relay_only += 1;
                }
            }
            _ => panic!("simulation must have bounded TTL"),
        }
        if cfg.links == Links::ExclusiveWifiCellBle
            && devices[message.source].radios.ble != devices[message.target].radios.ble
        {
            assert_ne!(message.state.phase(), Phase::RecipientAccepted,
                "exclusive single-radio model must not invent BLE-to-IP gateways");
        }
        assert!(message.holders.len() <= MAX_COPIES);
    }
    assert_eq!(
        summary.recipient_accepted + summary.revoked + summary.expired,
        summary.messages
    );
    delivery_times.sort_unstable();
    if !delivery_times.is_empty() {
        let n = delivery_times.len();
        summary.p50_delivery_ticks = Some(delivery_times[(n - 1) / 2]);
        summary.p95_delivery_ticks = Some(delivery_times[(n * 95).div_ceil(100) - 1]);
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_population_and_staircases_are_exact() {
        let cfg = Scenario::baseline(Links::BleOnly, Movement::Stationary, Outages::None);
        let devices: Vec<_> = (0..DEVICE_COUNT).map(|i| Device::new(i, cfg)).collect();
        for floor in 0..FLOOR_COUNT {
            assert_eq!(devices.iter().filter(|d| d.position.floor as usize == floor).count(), 400);
            let relays = devices.iter().filter(|d| {
                d.position.floor as usize == floor && d.may_forward
            }).count();
            assert!((200..390).contains(&relays), "relay consent must not depend on floor");
            for room in 0..ROOMS_PER_FLOOR {
                let position = Position::room(floor as u8, room as u8);
                let occupants = devices.iter().filter(|d| d.position == position).count();
                assert!((5..=6).contains(&occupants));
            }
        }
        assert_eq!(STAIR_X.len() * (FLOOR_COUNT - 1), 20);
        assert!(STAIR_X.windows(2).all(|pair| pair[1] > pair[0]));
    }

    #[test]
    fn motion_uses_hallways_and_adjacent_stair_floors() {
        let cfg = Scenario::baseline(Links::BleOnly, Movement::All, Outages::None);
        for id in 0..DEVICE_COUNT {
            let mut device = Device::new(id, cfg);
            for tick in 0..72 {
                let previous = device.position;
                device.advance(tick, cfg.seed);
                let current = device.position;
                assert!((current.floor as usize) < FLOOR_COUNT && current.x < HALL_SLOTS as u8);
                assert!(current.floor.abs_diff(previous.floor) <= 1);
                assert!(current.x.abs_diff(previous.x) <= 1);
                if current.floor != previous.floor {
                    assert_eq!(current.x, previous.x);
                    assert!(STAIR_X.contains(&current.x));
                    assert!(matches!(current.zone, Zone::Stair(_)));
                }
                if previous.zone == Zone::Room {
                    assert_eq!(current.x, previous.x);
                    assert_eq!(current.floor, previous.floor);
                }
            }
        }
    }

    #[test]
    fn wifi_without_internet_still_uses_shared_local_lan_assumption() {
        let cfg = Scenario::baseline(Links::SharedWifi, Movement::Stationary, Outages::None);
        let a = Device::new(0, cfg);
        let b = Device::new(1999, cfg);
        assert!(can_send(&a, &b, 0, cfg));
    }

    #[test]
    fn no_ble_to_ip_bridge_in_exclusive_radio_mode() {
        let cfg = Scenario::baseline(Links::ExclusiveWifiCellBle, Movement::All, Outages::None);
        let devices: Vec<_> = (0..DEVICE_COUNT).map(|i| Device::new(i, cfg)).collect();
        assert!(devices.iter().all(|d| {
            let r = d.radios;
            u8::from(r.wifi) + u8::from(r.cell) + u8::from(r.ble) == 1
        }));
        for tick in [0, 12, 55] {
            for i in [0, 1, 2, 36, 1978] {
                for j in [0, 1, 2, 348, 1999] {
                    if devices[i].radios.ble != devices[j].radios.ble {
                        assert!(!can_send(&devices[i], &devices[j], tick, cfg));
                    }
                }
            }
        }
    }

    #[test]
    fn replay_is_seed_deterministic_and_copies_are_bounded() {
        let mut cfg = Scenario::baseline(Links::MixedWifiBle, Movement::Quarter, Outages::Repeated);
        cfg.messages = 40;
        let first = run(cfg);
        assert_eq!(first, run(cfg));
        assert!(first.max_copies_observed <= MAX_COPIES);
        assert_eq!(first.messages, first.recipient_accepted + first.expired + first.revoked);
    }

    #[test]
    fn messages_created_during_a_wifi_outage_are_retried_after_return() {
        let normal = run(Scenario::baseline(Links::SharedWifi, Movement::Stationary, Outages::None));
        let outage = run(Scenario::baseline(Links::SharedWifi, Movement::Stationary, Outages::SingleWindow));
        assert_eq!(normal.recipient_accepted, normal.messages);
        assert_eq!(outage.recipient_accepted, outage.messages);
        assert!(outage.total_delivery_ticks > normal.total_delivery_ticks,
            "offline window must delay newly created messages");
    }

    #[test]
    fn synthetic_ble_carry_by_moving_devices_improves_contact_opportunities() {
        let static_case = run(Scenario::baseline(Links::BleOnly, Movement::Stationary, Outages::None));
        let moving_case = run(Scenario::baseline(Links::BleOnly, Movement::All, Outages::None));
        assert!(moving_case.recipient_accepted > static_case.recipient_accepted,
            "walking carriers should help the calibrated-future contact model");
    }

    #[test]
    fn entire_four_by_three_by_three_matrix_uses_real_akula_state_machine() {
        let links = [
            Links::SharedWifi,
            Links::MixedWifiBle,
            Links::BleOnly,
            Links::ExclusiveWifiCellBle,
        ];
        let movements = [Movement::Stationary, Movement::Quarter, Movement::All];
        let outages = [Outages::None, Outages::SingleWindow, Outages::Repeated];
        let mut families = 0;
        let mut totals = [0usize; 4];
        for (mode, link) in links.into_iter().enumerate() {
            for movement in movements {
                for outage in outages {
                    let cfg = Scenario::baseline(link, movement, outage);
                    let result = run(cfg);
                    println!(
                        "AK-MSG-SIM-2000 mode={link:?} move={movement:?} offline={outage:?} \
                         recipients={} expired={} revoked={} copies={} assumed_bytes={} \
                         forbidden_pairs={} late={} total_delivery_ticks={} p50_ticks={:?} p95_ticks={:?}",
                        result.recipient_accepted, result.expired, result.revoked,
                        result.copied_to_relays, result.assumed_bytes_transferred,
                        result.forbidden_cross_technology_pairs,
                        result.late_deliveries, result.total_delivery_ticks,
                        result.p50_delivery_ticks, result.p95_delivery_ticks,
                    );
                    assert!(result.max_copies_observed <= MAX_COPIES);
                    totals[mode] += result.recipient_accepted;
                    families += 1;
                }
            }
        }
        assert_eq!(families, 36);
        assert!(totals[0] > 0, "shared Wi-Fi must allow test delivery");
    }
}
