//! Two pure questions, answered in one place.
//!
//! 1. **What type is this device?** ArcScan detects one; the operator may
//!    disagree and say so. [`effective_type`] settles which of the two the
//!    interface, the filters and the exports use, and records which it was.
//! 2. **Is the evidence behind the detection still current?** ArcScan is not a
//!    monitor; it learns only when a scan runs. [`freshness`] turns a count of
//!    qualifying misses into one of three words, and [`cap_for_freshness`] says
//!    what a stale answer is still allowed to claim.
//!
//! Nothing here touches a socket or a database. Both rules are used by the
//! backend for exports and diagnostics, and mirrored in
//! `src/lib/effectiveType.ts` for the interface — the same arrangement the
//! display-name rule has had since v1.7.

use serde::{Deserialize, Serialize};

use super::model::{Confidence, DeviceType};

/// How many consecutive qualifying discovery scans must fail to re-observe a
/// piece of evidence before ArcScan stops leaning on it.
///
/// Three, and not a setting. One miss is ordinary — multicast is lossy and a
/// device asleep for four seconds answers nothing. Two is a coincidence worth
/// noticing. Three completed, discovery-capable scans that all reached the
/// device and all heard nothing is a pattern, and the number is small enough
/// that a weekly scanner reaches it inside a month rather than never.
///
/// A *setting* would be worse than a constant: the number only means anything
/// alongside the definition of a qualifying miss, and exposing one without the
/// other invites a person to turn it down to 1 and then disbelieve the result.
pub const STALE_AFTER_MISSES: i64 = 3;

/// Where the type on screen came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeSource {
    /// The operator chose it. Includes an explicit choice of Unknown.
    User,
    /// ArcScan's own reading of the evidence.
    Automatic,
}

impl TypeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TypeSource::User => "user",
            TypeSource::Automatic => "automatic",
        }
    }
}

/// The answer to "what type is this device, and who decided?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveType {
    pub effective_type: DeviceType,
    pub type_source: TypeSource,
    /// What ArcScan detected, kept underneath whatever the operator chose so
    /// the drawer can show both and clearing the override is not a loss.
    pub detected_type: DeviceType,
    pub detected_confidence: Confidence,
}

/// Settle the type for one device.
///
/// The whole rule: an override wins, and `None` means Auto. An *explicit*
/// `Some(DeviceType::Unknown)` is a different thing from `None` — it is a
/// person saying "ArcScan is wrong and I do not know either", which is a real
/// answer and must not be silently re-detected on the next scan.
///
/// Confidence belongs to the detection, never to the override: ArcScan has no
/// business grading how sure the operator is.
pub fn effective_type(
    detected_type: DeviceType,
    detected_confidence: Confidence,
    user_override: Option<DeviceType>,
) -> EffectiveType {
    match user_override {
        Some(chosen) => EffectiveType {
            effective_type: chosen,
            type_source: TypeSource::User,
            detected_type,
            detected_confidence,
        },
        None => EffectiveType {
            effective_type: detected_type,
            type_source: TypeSource::Automatic,
            detected_type,
            detected_confidence,
        },
    }
}

/// How current the evidence behind a claim is.
///
/// Deliberately counted in *scans*, not in days. ArcScan runs when a person
/// runs it; a laptop that scanned in March and again in September has not
/// learned anything in between, and letting the calendar age evidence would
/// mean punishing a device for its owner's holiday. Wall-clock age is shown as
/// context ("last seen 4 discovery scans ago" alongside a date), never used to
/// decide anything.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    /// Re-observed by the most recent qualifying scan.
    #[default]
    Current,
    /// Missed by one or two qualifying scans. Still believed, and said to be
    /// getting old.
    Aging,
    /// Missed by [`STALE_AFTER_MISSES`] consecutive qualifying scans. Kept,
    /// shown and dated, but no longer able to carry a High-confidence claim on
    /// its own.
    Stale,
}

impl Freshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Freshness::Current => "current",
            Freshness::Aging => "aging",
            Freshness::Stale => "stale",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "aging" => Freshness::Aging,
            "stale" => Freshness::Stale,
            _ => Freshness::Current,
        }
    }

    pub fn is_stale(self) -> bool {
        self == Freshness::Stale
    }
}

/// Turn a miss count into a state. A negative or absent count reads as current,
/// because "never counted" is not the same as "counted and missed".
pub fn freshness(misses: i64) -> Freshness {
    if misses <= 0 {
        Freshness::Current
    } else if misses < STALE_AFTER_MISSES {
        Freshness::Aging
    } else {
        Freshness::Stale
    }
}

/// What a claim resting entirely on stale evidence is still allowed to say.
///
/// High confidence means "the device declared this through a protocol built for
/// the purpose, and something independent agrees". Neither half of that sentence
/// survives three qualifying scans in which nothing was heard, so High is capped
/// to Medium. It is not dropped further: the evidence was real, it is still on
/// file, and a printer that has stopped advertising is still overwhelmingly
/// likely to be a printer.
///
/// Anything already at Medium or below is untouched — there is no downward
/// spiral, and a device does not decay to Unknown by being quiet.
pub fn cap_for_freshness(confidence: Confidence, evidence: Freshness) -> Confidence {
    if evidence.is_stale() && confidence == Confidence::High {
        Confidence::Medium
    } else {
        confidence
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_override_means_the_detected_answer_and_says_so() {
        let resolved = effective_type(DeviceType::Printer, Confidence::High, None);
        assert_eq!(resolved.effective_type, DeviceType::Printer);
        assert_eq!(resolved.type_source, TypeSource::Automatic);
        assert_eq!(resolved.detected_type, DeviceType::Printer);
        assert_eq!(resolved.detected_confidence, Confidence::High);
    }

    #[test]
    fn every_override_wins_and_keeps_the_detected_answer_underneath() {
        for chosen in DeviceType::ALL {
            let resolved =
                effective_type(DeviceType::MediaDevice, Confidence::Medium, Some(chosen));
            assert_eq!(resolved.effective_type, chosen);
            assert_eq!(resolved.type_source, TypeSource::User);
            // Clearing the override has to be able to reveal this again, so it
            // must survive being overridden.
            assert_eq!(resolved.detected_type, DeviceType::MediaDevice);
            assert_eq!(resolved.detected_confidence, Confidence::Medium);
        }
    }

    #[test]
    fn an_explicit_unknown_is_not_the_same_as_auto() {
        let auto = effective_type(DeviceType::Camera, Confidence::Medium, None);
        let chosen = effective_type(
            DeviceType::Camera,
            Confidence::Medium,
            Some(DeviceType::Unknown),
        );
        assert_eq!(auto.effective_type, DeviceType::Camera);
        assert_eq!(auto.type_source, TypeSource::Automatic);
        assert_eq!(chosen.effective_type, DeviceType::Unknown);
        assert_eq!(chosen.type_source, TypeSource::User);
        assert_ne!(auto.effective_type, chosen.effective_type);
    }

    #[test]
    fn clearing_an_override_reveals_the_current_automatic_answer() {
        // The detection is allowed to have moved on while the override stood.
        let overridden =
            effective_type(DeviceType::Nas, Confidence::High, Some(DeviceType::Router));
        assert_eq!(overridden.effective_type, DeviceType::Router);
        let cleared = effective_type(DeviceType::Nas, Confidence::High, None);
        assert_eq!(cleared.effective_type, DeviceType::Nas);
        assert_eq!(cleared.type_source, TypeSource::Automatic);
    }

    #[test]
    fn misses_map_onto_the_three_states_at_the_documented_boundaries() {
        assert_eq!(freshness(-1), Freshness::Current);
        assert_eq!(freshness(0), Freshness::Current);
        assert_eq!(freshness(1), Freshness::Aging);
        assert_eq!(freshness(2), Freshness::Aging);
        assert_eq!(freshness(STALE_AFTER_MISSES), Freshness::Stale);
        assert_eq!(freshness(STALE_AFTER_MISSES + 40), Freshness::Stale);
    }

    #[test]
    fn freshness_round_trips_through_its_wire_name() {
        for state in [Freshness::Current, Freshness::Aging, Freshness::Stale] {
            assert_eq!(Freshness::parse(state.as_str()), state);
        }
        assert_eq!(Freshness::parse("elderly"), Freshness::Current);
    }

    #[test]
    fn stale_evidence_cannot_carry_high_confidence_alone() {
        assert_eq!(
            cap_for_freshness(Confidence::High, Freshness::Stale),
            Confidence::Medium
        );
        // And nothing else moves: no decay spiral, no quiet slide to Unknown.
        for confidence in [Confidence::Medium, Confidence::Low, Confidence::Unknown] {
            assert_eq!(
                cap_for_freshness(confidence, Freshness::Stale),
                confidence,
                "{confidence:?} should not be reduced further"
            );
        }
        for state in [Freshness::Current, Freshness::Aging] {
            assert_eq!(cap_for_freshness(Confidence::High, state), Confidence::High);
        }
    }
}
