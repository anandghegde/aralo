//! Token counts per feature and per profile, and the soft caps that warn when
//! a profile passes them. A cap never stops a stream that is under way.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::model::{Feature, Usage};

#[derive(Debug, Default)]
pub struct Meter {
    inner: Mutex<Counters>,
}

#[derive(Debug, Default)]
struct Counters {
    usage: HashMap<(Feature, String), Usage>,
    caps: HashMap<String, u64>,
}

/// A profile just went past its soft cap. Reported once, at the crossing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapWarning {
    pub profile: String,
    pub used: u64,
    pub cap: u64,
}

impl Meter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds usage to a feature's count for a profile. Returns a warning when
    /// this is the addition that takes the profile's total past its cap.
    pub fn record(&self, feature: Feature, profile: &str, usage: Usage) -> Option<CapWarning> {
        let mut counters = self.lock();
        let before = counters.profile_total(profile);
        *counters
            .usage
            .entry((feature, profile.to_owned()))
            .or_default() += usage;
        let after = before + usage.total();
        let cap = *counters.caps.get(profile)?;
        (before <= cap && after > cap).then(|| CapWarning {
            profile: profile.to_owned(),
            used: after,
            cap,
        })
    }

    pub fn usage(&self, feature: Feature, profile: &str) -> Usage {
        self.lock()
            .usage
            .get(&(feature, profile.to_owned()))
            .copied()
            .unwrap_or_default()
    }

    /// Input plus output tokens across every feature.
    pub fn profile_total(&self, profile: &str) -> u64 {
        self.lock().profile_total(profile)
    }

    /// Sets or clears a profile's soft cap, in tokens.
    pub fn set_soft_cap(&self, profile: &str, cap: Option<u64>) {
        let mut counters = self.lock();
        match cap {
            Some(cap) => counters.caps.insert(profile.to_owned(), cap),
            None => counters.caps.remove(profile),
        };
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Counters> {
        // Counters stay consistent even if a holder panicked: every update is
        // a single insert.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Counters {
    fn profile_total(&self, profile: &str) -> u64 {
        self.usage
            .iter()
            .filter(|((_, name), _)| name == profile)
            .map(|(_, usage)| usage.total())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(input: u64, output: u64) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
        }
    }

    #[test]
    fn counts_are_kept_per_feature_and_per_profile() {
        let meter = Meter::new();
        meter.record(Feature::Command, "openai", used(10, 5));
        meter.record(Feature::Command, "openai", used(1, 1));
        meter.record(Feature::Block, "openai", used(100, 0));
        meter.record(Feature::Command, "ollama", used(7, 7));
        assert_eq!(meter.usage(Feature::Command, "openai"), used(11, 6));
        assert_eq!(meter.usage(Feature::Block, "openai"), used(100, 0));
        assert_eq!(meter.profile_total("openai"), 117);
        assert_eq!(meter.profile_total("ollama"), 14);
    }

    #[test]
    fn a_soft_cap_warns_once_at_the_crossing() {
        let meter = Meter::new();
        meter.set_soft_cap("openai", Some(100));
        assert_eq!(meter.record(Feature::Command, "openai", used(60, 40)), None);
        assert_eq!(
            meter.record(Feature::Block, "openai", used(1, 0)),
            Some(CapWarning {
                profile: "openai".into(),
                used: 101,
                cap: 100
            })
        );
        assert_eq!(meter.record(Feature::Block, "openai", used(50, 50)), None);
        assert_eq!(meter.record(Feature::Block, "ollama", used(500, 500)), None);
    }
}
