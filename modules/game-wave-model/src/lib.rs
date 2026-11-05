//! Deterministic, data-driven game-wave scheduler.
//!
//! This is independently written research code, not a port or copy of any
//! third-party game. In particular, no Tetraform XML, assets or scripts ship
//! here. See README.md for the limits of compatibility evidence.
#![forbid(unsafe_code)]

use std::collections::HashSet;

const MAX_WAVES: usize = 1_024;
const MAX_GROUPS: usize = 4_096;
const MAX_ENTRIES: usize = 16_384;
const MAX_TOTAL_GROUPS: usize = 4_096;
const MAX_TOTAL_ENTRIES: usize = 65_536;
const MAX_ALIVE: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpawnKind {
    Enemy,
    PowerUp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Spawn {
    pub kind: SpawnKind,
    pub archetype: String,
}

impl Spawn {
    pub fn enemy(name: &str) -> Self {
        Self { kind: SpawnKind::Enemy, archetype: name.into() }
    }
    pub fn power_up(name: &str) -> Self {
        Self { kind: SpawnKind::PowerUp, archetype: name.into() }
    }
}

#[derive(Clone, Debug)]
pub enum Group {
    /// Resume the next group only when the counter becomes greater than ticks.
    Pause { ticks: u64 },
    /// First item on the first update, then every frequency+1 updates.
    Batch { frequency: u64, items: Vec<Spawn> },
    /// Historical behavior: a cycling group is disabled once all PRIOR groups
    /// are complete. The cycle counter can reach zero and then become negative.
    Cycle { frequency: u64, rest: u64, repeats: u32, items: Vec<Spawn> },
}

#[derive(Clone, Debug)]
pub struct Wave {
    pub name: String,
    /// Once all groups have *started*, force completion if wait > timeout.
    /// None and Some(0) both mean no forced timeout.
    pub timeout: Option<u64>,
    pub groups: Vec<Group>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    NoWaves,
    TooManyWaves,
    TooManyGroups { wave: usize },
    TooManyEntries { wave: usize, group: usize },
    TooManyTotalGroups,
    TooManyTotalEntries,
    EmptyArchetype { wave: usize, group: usize },
    BadShuffleIndex { upper: usize, chosen: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Spawned { wave: usize, group: usize, id: u64, item: Spawn },
    WaveChanged { wave: usize, name: String },
    Finished,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TickError {
    IdsExhausted,
    LiveBudgetExceeded,
}

#[derive(Debug)]
enum Mode {
    Pause { ticks: u64 },
    Batch { frequency: u64 },
    Cycle { frequency: u64, rest: u64, repeats_left: i64, enabled: bool },
}

#[derive(Debug)]
struct RunningGroup {
    mode: Mode,
    items: Vec<Spawn>,
    position: usize,
    clock: u64,
    // Last spawned object in each slot; reused slots overwrite this handle.
    last_ids: Vec<Option<u64>>,
}

impl RunningGroup {
    fn new(
        raw: Group,
        wave: usize,
        group: usize,
        choose: &mut impl FnMut(usize) -> usize,
    ) -> Result<Self, ConfigError> {
        let (mode, mut items) = match raw {
            Group::Pause { ticks } => (Mode::Pause { ticks }, Vec::new()),
            Group::Batch { frequency, items } => (Mode::Batch { frequency }, items),
            Group::Cycle { frequency, rest, repeats, items } => (
                Mode::Cycle { frequency, rest, repeats_left: i64::from(repeats), enabled: true },
                items,
            ),
        };
        if items.len() > MAX_ENTRIES {
            return Err(ConfigError::TooManyEntries { wave, group });
        }
        if items.iter().any(|item| item.archetype.is_empty()) {
            return Err(ConfigError::EmptyArchetype { wave, group });
        }
        // Original Flash group initialization shuffled each expanded group.
        // A caller-supplied chooser makes ordering reproducible without
        // claiming to reproduce Flash Player's Math.random implementation.
        for i in (1..items.len()).rev() {
            let j = choose(i + 1);
            if j > i {
                return Err(ConfigError::BadShuffleIndex { upper: i + 1, chosen: j });
            }
            items.swap(i, j);
        }
        let last_ids = vec![None; items.len()];
        // Historical Group constructor sets timer=frequency. The first
        // item is therefore emitted on the very first update, regardless
        // of the interval between later items.
        let initial_clock = match &mode {
            Mode::Pause { .. } => 0,
            Mode::Batch { frequency } | Mode::Cycle { frequency, .. } => *frequency,
        };
        Ok(Self { mode, items, position: 0, clock: initial_clock, last_ids })
    }

    fn can_advance(&self) -> bool {
        match self.mode {
            Mode::Pause { ticks } => self.clock > ticks,
            Mode::Batch { .. } => self.position >= self.items.len(),
            Mode::Cycle { enabled, .. } => !enabled && self.position >= self.items.len(),
        }
    }

    fn completed(&self, alive: &HashSet<u64>) -> bool {
        match self.mode {
            Mode::Pause { ticks } => self.clock > ticks,
            _ => self.position >= self.items.len()
                && self.last_ids.iter().all(|id| id.is_none_or(|id| !alive.contains(&id))),
        }
    }

    fn update(&mut self) -> Option<(usize, Spawn)> {
        self.clock = self.clock.saturating_add(1);
        match &mut self.mode {
            Mode::Pause { .. } => return None,
            Mode::Cycle { rest, repeats_left, enabled, .. } => {
                if *enabled && self.position >= self.items.len() && self.clock > *rest {
                    // Deliberately model the observable pre-decrement behavior
                    // of the historical counter, including zero -> negative.
                    *repeats_left = repeats_left.saturating_sub(1);
                    if *repeats_left != 0 {
                        self.position = 0;
                        self.clock = 0;
                    }
                }
            }
            Mode::Batch { .. } => {}
        }
        let frequency = match self.mode {
            Mode::Pause { .. } => unreachable!(),
            Mode::Batch { frequency } | Mode::Cycle { frequency, .. } => frequency,
        };
        if self.clock > frequency && self.position < self.items.len() {
            self.clock = 0;
            let slot = self.position;
            self.position += 1;
            Some((slot, self.items[slot].clone()))
        } else {
            None
        }
    }
}

#[derive(Debug)]
struct RunningWave {
    name: String,
    timeout: Option<u64>,
    groups: Vec<RunningGroup>,
    cursor: usize,
    wait: u64,
}

impl RunningWave {
    fn completed(&self, alive: &HashSet<u64>) -> bool {
        if self.cursor != self.groups.len() {
            return false;
        }
        if self.timeout.is_some_and(|t| t > 0 && self.wait > t) {
            return true;
        }
        self.groups.iter().all(|group| group.completed(alive))
    }

    fn update(&mut self, alive: &HashSet<u64>) -> Option<(usize, usize, Spawn)> {
        if self.cursor == self.groups.len() {
            self.wait = self.wait.saturating_add(1);
            return None;
        }
        if self.groups[self.cursor].can_advance() {
            self.cursor += 1;
            if self.cursor == self.groups.len() {
                return None;
            }
        }
        let index = self.cursor;
        let spawned = self.groups[index].update();
        // Cycling is controlled by the life of PREVIOUS groups, not the
        // currently spawned group. This distinction matters for old AS3 games.
        if (0..index).all(|i| self.groups[i].completed(alive)) {
            if let Mode::Cycle { enabled, .. } = &mut self.groups[index].mode {
                *enabled = false;
            }
        }
        spawned.map(|(slot, item)| (index, slot, item))
    }
}

/// One call to tick() is one logical Flash update, not one elapsed millisecond.
/// A host owns rendered objects, collision physics, and external dependencies.
#[derive(Debug)]
pub struct Scheduler {
    waves: Vec<RunningWave>,
    current: usize,
    transition_clock: Option<u64>,
    next_id: u64,
    alive: HashSet<u64>,
    finished: bool,
}

impl Scheduler {
    /// Deterministic original order. For testable reordering use with_shuffle.
    pub fn new(waves: Vec<Wave>) -> Result<Self, ConfigError> {
        Self::with_shuffle(waves, |n| n - 1)
    }

    pub fn with_shuffle(
        waves: Vec<Wave>,
        mut choose: impl FnMut(usize) -> usize,
    ) -> Result<Self, ConfigError> {
        if waves.is_empty() {
            return Err(ConfigError::NoWaves);
        }
        if waves.len() > MAX_WAVES {
            return Err(ConfigError::TooManyWaves);
        }
        let mut parsed = Vec::with_capacity(waves.len());
        let mut total_groups = 0usize;
        let mut total_entries = 0usize;
        for (wi, wave) in waves.into_iter().enumerate() {
            if wave.groups.len() > MAX_GROUPS {
                return Err(ConfigError::TooManyGroups { wave: wi });
            }
            total_groups = total_groups.checked_add(wave.groups.len())
                .ok_or(ConfigError::TooManyTotalGroups)?;
            if total_groups > MAX_TOTAL_GROUPS {
                return Err(ConfigError::TooManyTotalGroups);
            }
            let mut groups = Vec::with_capacity(wave.groups.len());
            for (gi, group) in wave.groups.into_iter().enumerate() {
                let n = match &group {
                    Group::Pause { .. } => 0,
                    Group::Batch { items, .. } | Group::Cycle { items, .. } => items.len(),
                };
                total_entries = total_entries.checked_add(n)
                    .ok_or(ConfigError::TooManyTotalEntries)?;
                if total_entries > MAX_TOTAL_ENTRIES {
                    return Err(ConfigError::TooManyTotalEntries);
                }
                groups.push(RunningGroup::new(group, wi, gi, &mut choose)?);
            }
            parsed.push(RunningWave {
                name: wave.name, timeout: wave.timeout, groups, cursor: 0, wait: 0,
            });
        }
        Ok(Self {
            waves: parsed, current: 0, transition_clock: None,
            next_id: 1, alive: HashSet::new(), finished: false,
        })
    }

    pub fn tick(&mut self) -> Result<Vec<Event>, TickError> {
        if self.finished {
            return Ok(Vec::new());
        }
        // A cycling group can spawn indefinitely without the host marking
        // objects gone. Halt without advancing the scheduler at the limit.
        if self.alive.len() >= MAX_ALIVE {
            return Err(TickError::LiveBudgetExceeded);
        }
        let mut events = Vec::new();
        if self.waves[self.current].completed(&self.alive)
            || self.transition_clock.is_some()
        {
            self.transition_clock = match self.transition_clock {
                None => Some(0),
                Some(t) if t >= 149 => {
                    self.current += 1;
                    if self.current == self.waves.len() {
                        self.finished = true;
                        events.push(Event::Finished);
                        return Ok(events);
                    }
                    events.push(Event::WaveChanged {
                        wave: self.current,
                        name: self.waves[self.current].name.clone(),
                    });
                    None
                }
                Some(t) => Some(t + 1),
            };
        }
        if self.transition_clock.is_none() {
            if let Some((group, slot, item)) = self.waves[self.current].update(&self.alive) {
                let id = self.next_id;
                self.next_id = self.next_id.checked_add(1).ok_or(TickError::IdsExhausted)?;
                // Older objects can still be alive in the game world even
                // after the original spawn slot is overwritten by a cycle.
                // Only completion checks the latest handle for each slot.
                self.waves[self.current].groups[group].last_ids[slot] = Some(id);
                self.alive.insert(id);
                events.push(Event::Spawned { wave: self.current, group, id, item });
            }
        }
        Ok(events)
    }

    /// Report that the host removed/killed a spawned object. Unknown and
    /// already-replaced ids return false rather than affecting another spawn.
    pub fn mark_gone(&mut self, id: u64) -> bool {
        self.alive.remove(&id)
    }

    pub fn active_count(&self) -> usize {
        self.alive.len()
    }

    pub fn current_wave(&self) -> Option<usize> {
        (!self.finished).then_some(self.current)
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }
}
