//! The concurrency governor (master plan S14.4): how many agents may run at
//! once, globally, per project, and per provider.
//!
//! Why a governor rather than an unbounded `join_all`: every node runs a real
//! coding-agent CLI. Spawning the ready set of an enterprise-feature run
//! without a limit means four or five concurrent agents hammering the same
//! machine, the same provider account, and possibly the same repository --
//! which produces rate-limit failures, disk pressure, and timeouts that look
//! like agent failures rather than what they are. The governor makes the
//! limit explicit, configurable, and *visible* (the engine reports what it is
//! holding back instead of appearing to hang).
//!
//! Ordering is the engine's job, not the governor's: the engine dispatches
//! its ready set in template order and re-examines the whole set after every
//! completion, so a node that was refused a slot is retried before any newer
//! node -- which is the fairness property without a queue inside the
//! governor. What lives here is only the accounting, because accounting that
//! drifts is the failure mode that matters: the caps silently stop holding
//! and nothing says so.

use std::collections::HashMap;

use nacc_domain::{ProjectId, ProviderId};

/// Where a slot is charged. `project` always applies; `provider` applies when
/// the role resolved to one (an unassigned role is charged globally and
/// per-project only).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub struct SlotKey {
    pub project: ProjectId,
    pub provider: Option<ProviderId>,
}

impl SlotKey {
    pub const fn new(project: ProjectId, provider: Option<ProviderId>) -> Self {
        Self { project, provider }
    }
}

/// The configured ceilings. Defaults are deliberately conservative: NACC is a
/// desktop application on a machine a human is also using.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ConcurrencyLimits {
    /// Total concurrent nodes across every project and provider.
    pub global: usize,
    /// Concurrent nodes within one project.
    pub per_project: usize,
    /// Concurrent nodes against one provider (an account, effectively).
    pub per_provider: usize,
}

impl Default for ConcurrencyLimits {
    fn default() -> Self {
        Self {
            global: 4,
            per_project: 2,
            per_provider: 2,
        }
    }
}

impl ConcurrencyLimits {
    /// Clamp away the values that would disable the governor. A zero limit is
    /// not "unlimited" -- treating it as such is how a config typo turns into
    /// an unbounded fan-out of agent processes. `usize::MAX` is allowed
    /// through as the explicit way to say "no ceiling here".
    pub fn normalized(self) -> Self {
        Self {
            global: self.global.max(1),
            per_project: self.per_project.max(1),
            per_provider: self.per_provider.max(1),
        }
    }
}

/// Why a request could not be admitted right now.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Capacity {
    Global,
    Project,
    Provider,
}

/// A granted slot. Holding one is the only way to be charged, and releasing
/// one consumes it -- a double release is a compile error rather than a
/// silently inflated limit.
#[derive(Debug, Eq, PartialEq)]
pub struct Permit {
    key: SlotKey,
}

impl Permit {
    pub const fn key(&self) -> SlotKey {
        self.key
    }
}

/// The live bookkeeping: what is in flight, against which caps.
#[derive(Debug)]
pub struct Governor {
    limits: ConcurrencyLimits,
    in_flight: HashMap<SlotKey, usize>,
}

impl Governor {
    pub fn new(limits: ConcurrencyLimits) -> Self {
        Self {
            limits: limits.normalized(),
            in_flight: HashMap::new(),
        }
    }

    pub fn limits(&self) -> ConcurrencyLimits {
        self.limits
    }

    /// How many nodes are running under this governor right now.
    pub fn in_flight(&self) -> usize {
        self.in_flight.values().sum()
    }

    pub fn in_flight_for(&self, key: SlotKey) -> usize {
        self.in_flight.get(&key).copied().unwrap_or(0)
    }

    /// The current bottleneck for `key`, if it cannot be admitted now.
    pub fn capacity_for(&self, key: SlotKey) -> Option<Capacity> {
        if self.in_flight() >= self.limits.global {
            return Some(Capacity::Global);
        }
        let project_in_flight: usize = self
            .in_flight
            .iter()
            .filter(|(slot, _)| slot.project == key.project)
            .map(|(_, count)| count)
            .sum();
        if project_in_flight >= self.limits.per_project {
            return Some(Capacity::Project);
        }
        if let Some(provider) = key.provider {
            let provider_in_flight: usize = self
                .in_flight
                .iter()
                .filter(|(slot, _)| slot.provider == Some(provider))
                .map(|(_, count)| count)
                .sum();
            if provider_in_flight >= self.limits.per_provider {
                return Some(Capacity::Provider);
            }
        }
        None
    }

    /// Admit `key` if there is room right now.
    pub fn try_acquire(&mut self, key: SlotKey) -> Option<Permit> {
        if self.capacity_for(key).is_some() {
            return None;
        }
        *self.in_flight.entry(key).or_insert(0) += 1;
        Some(Permit { key })
    }

    /// Give a slot back.
    pub fn release(&mut self, permit: Permit) {
        match self.in_flight.get_mut(&permit.key) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                self.in_flight.remove(&permit.key);
            }
            // Unreachable: a `Permit` can only be produced by `try_acquire`.
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> ProjectId {
        ProjectId::new()
    }

    #[test]
    fn a_zero_limit_is_clamped_rather_than_read_as_unlimited() {
        let limits = ConcurrencyLimits {
            global: 0,
            per_project: 0,
            per_provider: 0,
        }
        .normalized();
        assert_eq!(limits.global, 1);
        assert_eq!(limits.per_project, 1);
        assert_eq!(limits.per_provider, 1);
        let mut governor = Governor::new(limits);
        let key = SlotKey::new(project(), Some(ProviderId::Claude));
        assert!(governor.try_acquire(key).is_some());
        assert!(governor.try_acquire(key).is_none());
    }

    #[test]
    fn the_global_cap_holds_even_when_every_per_key_cap_has_room() {
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 2,
            per_project: 10,
            per_provider: 10,
        });
        let project = project();
        let p1 = SlotKey::new(project, Some(ProviderId::Claude));
        let p2 = SlotKey::new(project, Some(ProviderId::Codex));
        let p3 = SlotKey::new(project, None);
        assert!(governor.try_acquire(p1).is_some());
        assert!(governor.try_acquire(p2).is_some());
        assert_eq!(governor.capacity_for(p3), Some(Capacity::Global));
        assert!(governor.try_acquire(p3).is_none());
        assert_eq!(governor.in_flight(), 2);
    }

    #[test]
    fn the_per_project_cap_is_charged_across_providers() {
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 10,
            per_project: 1,
            per_provider: 10,
        });
        let shared = project();
        let first = SlotKey::new(shared, Some(ProviderId::Claude));
        let second = SlotKey::new(shared, Some(ProviderId::Codex));
        let other_project = SlotKey::new(project(), Some(ProviderId::Claude));
        assert!(governor.try_acquire(first).is_some());
        assert_eq!(governor.capacity_for(second), Some(Capacity::Project));
        // A different project still fits: the per-project cap must not leak
        // across projects.
        assert!(governor.try_acquire(other_project).is_some());
    }

    #[test]
    fn the_per_provider_cap_is_charged_across_projects() {
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 10,
            per_project: 10,
            per_provider: 1,
        });
        let a = SlotKey::new(project(), Some(ProviderId::Claude));
        let b = SlotKey::new(project(), Some(ProviderId::Claude));
        let c = SlotKey::new(project(), Some(ProviderId::Codex));
        assert!(governor.try_acquire(a).is_some());
        assert_eq!(governor.capacity_for(b), Some(Capacity::Provider));
        assert!(governor.try_acquire(c).is_some());
    }

    #[test]
    fn an_unassigned_provider_is_not_charged_against_any_provider_cap() {
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 10,
            per_project: 10,
            per_provider: 1,
        });
        let project = project();
        assert!(governor
            .try_acquire(SlotKey::new(project, Some(ProviderId::Claude)))
            .is_some());
        // Two nodes with no resolved provider are limited only by the other
        // caps -- there is no provider account for them to exhaust.
        assert!(governor.try_acquire(SlotKey::new(project, None)).is_some());
        assert!(governor.try_acquire(SlotKey::new(project, None)).is_some());
    }

    #[test]
    fn releasing_a_slot_makes_room_again() {
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 1,
            per_project: 1,
            per_provider: 1,
        });
        let project = project();
        let first = SlotKey::new(project, Some(ProviderId::Claude));
        let second = SlotKey::new(project, Some(ProviderId::Codex));
        let permit = governor.try_acquire(first).expect("global=1 admits one");
        assert!(governor.try_acquire(second).is_none());
        governor.release(permit);
        assert_eq!(governor.in_flight(), 0);
        // The slot freed globally is usable by a *different* key: limits are
        // ceilings, not reservations.
        let permit = governor
            .try_acquire(second)
            .expect("the slot is free again");
        governor.release(permit);
        assert_eq!(governor.in_flight(), 0);
    }

    #[test]
    fn in_flight_counting_survives_nested_acquisition_of_one_key() {
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 4,
            per_project: 4,
            per_provider: 4,
        });
        let key = SlotKey::new(project(), Some(ProviderId::Claude));
        let first = governor.try_acquire(key).unwrap();
        let second = governor.try_acquire(key).unwrap();
        assert_eq!(governor.in_flight_for(key), 2);
        governor.release(first);
        assert_eq!(governor.in_flight_for(key), 1);
        governor.release(second);
        assert_eq!(governor.in_flight(), 0);
    }

    #[test]
    fn order_of_dispatch_is_the_callers_decision() {
        // The governor deliberately does not queue: the engine re-examines
        // its ready set in template order after every completion, which keeps
        // the oldest node first without a second scheduler inside this type.
        let mut governor = Governor::new(ConcurrencyLimits {
            global: 2,
            per_project: 2,
            per_provider: 2,
        });
        let project = project();
        let keys: Vec<SlotKey> = (0..3)
            .map(|_| SlotKey::new(project, Some(ProviderId::Claude)))
            .collect();
        let first = governor.try_acquire(keys[0]).unwrap();
        let second = governor.try_acquire(keys[1]).unwrap();
        assert!(
            governor.try_acquire(keys[2]).is_none(),
            "three is over the cap"
        );
        governor.release(second);
        assert!(
            governor.try_acquire(keys[2]).is_some(),
            "the engine's oldest-first retry gets the slot"
        );
        governor.release(first);
        assert_eq!(governor.in_flight(), 1);
    }
}
