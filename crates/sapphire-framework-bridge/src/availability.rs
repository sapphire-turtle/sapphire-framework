//! How much of the time this bridge is running (#190), as a tier the election ranks by.
//!
//! Once a minute the bridge credits the time since the last tick to the day it is in, in
//! `<bridge dir>/availability.toml`. Only time the process actually ran counts: the credit
//! is the monotonic clock's progress, never more than the wall clock's, and never more
//! than two ticks — so a host that slept (the wall clock ran on, the monotonic clock did
//! not, or a tick came very late) is not credited for the sleep. Time the bridge was not
//! running is never credited at all.
//!
//! The tier is announced in Hello and never written to the ledger: every reachable device
//! can say its own, and a value that changes every few minutes has no place in a synced
//! file.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp, ToSpan};
use serde::{Deserialize, Serialize};

use crate::{Bridge, Error, Result};

/// How often the bridge records that it is running.
pub(crate) const TICK: Duration = Duration::from_secs(60);

/// How many days of history the tier is measured over, today included.
pub(crate) const WINDOW_DAYS: u64 = 7;

/// How little history yields no tier at all: an hour says nothing about a week.
const MIN_HISTORY: SignedDuration = SignedDuration::from_hours(1);

/// The tier of a share of time up: `≥ 99 %` → 3, `≥ 95 %` → 2, `≥ 80 %` → 1, below → 0.
///
/// Tiers rather than percentages, so small swings do not reorder candidates.
pub(crate) fn tier_of(up: f64) -> u8 {
    if up >= 0.99 {
        3
    } else if up >= 0.95 {
        2
    } else if up >= 0.80 {
        1
    } else {
        0
    }
}

/// How much of a tick's interval to credit: what the monotonic clock saw, capped by the
/// wall clock and by two ticks.
///
/// On Linux and macOS the monotonic clock stops while the host sleeps, so the cap by the
/// wall clock is what keeps a clock stepped back from crediting time twice. Where it does
/// not stop, a tick that comes far later than due — the host slept through it — is capped.
pub(crate) fn credit(monotonic: Duration, wall: SignedDuration) -> Duration {
    let wall = Duration::try_from(wall).unwrap_or(Duration::ZERO);
    monotonic.min(wall).min(TICK * 2)
}

/// The per-day record, as `availability.toml` holds it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct Record {
    /// When this host started recording: the start of its history.
    since: Timestamp,
    /// Minutes up, per UTC day, for the last [`WINDOW_DAYS`] days.
    #[serde(default)]
    minutes: BTreeMap<Date, u32>,
    /// Seconds credited but not yet a whole minute. Not written: losing under a minute on
    /// a restart is nothing.
    #[serde(skip)]
    carry: Duration,
}

impl Record {
    /// An empty history starting at `now`.
    pub(crate) fn new(now: Timestamp) -> Record {
        Record {
            since: now,
            minutes: BTreeMap::new(),
            carry: Duration::ZERO,
        }
    }

    /// The record at `path`, or `None` when there is none yet.
    pub(crate) fn load(path: &Path) -> Result<Option<Record>> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .map(Some)
                .map_err(|e| Error::Format(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write it to `path`, replacing what was there.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let body = toml::to_string(self).map_err(|e| Error::Format(e.to_string()))?;
        crate::routes::write_atomic(
            path,
            "# How much of the time sapphire-bridge has been running here. Not synced.",
            &body,
        )
    }

    /// The UTC day `now` is in.
    fn day_of(now: Timestamp) -> Date {
        TimeZone::UTC.to_datetime(now).date()
    }

    /// The first day of the window ending on `now`'s day.
    fn window_start(now: Timestamp) -> Date {
        Record::day_of(now) - (WINDOW_DAYS as i64 - 1).days()
    }

    /// Credit `up` to `now`'s day, and drop the days the window has passed.
    pub(crate) fn add(&mut self, now: Timestamp, up: Duration) {
        self.carry += up;
        let whole = self.carry.as_secs() / 60;
        self.carry -= Duration::from_secs(whole * 60);
        if whole > 0 {
            let day = self.minutes.entry(Record::day_of(now)).or_default();
            // A day has 1440 minutes; anything above is a clock gone wrong.
            *day = day.saturating_add(whole as u32).min(24 * 60);
        }
        let start = Record::window_start(now);
        self.minutes.retain(|day, _| *day >= start);
    }

    /// The share of the window this host was up, with the history it covers.
    ///
    /// The window is the last [`WINDOW_DAYS`] UTC days up to `now`, starting no earlier
    /// than the history does.
    fn share(&self, now: Timestamp) -> Option<(f64, SignedDuration)> {
        let day_start = TimeZone::UTC
            .to_timestamp(Record::window_start(now).to_datetime(jiff::civil::Time::midnight()))
            .expect("a recent day's midnight");
        let from = day_start.max(self.since);
        let span = now.duration_since(from);
        if span < MIN_HISTORY {
            return None;
        }
        let up: u32 = self
            .minutes
            .range(Record::window_start(now)..)
            .map(|(_, m)| *m)
            .sum();
        let share = f64::from(up) / span.as_mins() as f64;
        Some((share.min(1.0), now.duration_since(self.since)))
    }

    /// The tier to announce at `now`, or `None` with too little history.
    ///
    /// A host with less than a full window of history reports one tier below its
    /// measurement, so a fresh install does not outrank a device with a long record.
    pub(crate) fn tier(&self, now: Timestamp) -> Option<u8> {
        let (share, history) = self.share(now)?;
        let tier = tier_of(share);
        let full = SignedDuration::from_hours(24 * WINDOW_DAYS as i64);
        Some(if history < full {
            tier.saturating_sub(1)
        } else {
            tier
        })
    }
}

/// Record this bridge's uptime, once a [`TICK`], for as long as it runs, and keep its
/// announced tier current.
pub(crate) async fn run(bridge: Arc<Bridge>) -> Result<()> {
    let path = bridge.dir.availability_toml();
    let mut record = match Record::load(&path) {
        Ok(Some(record)) => record,
        Ok(None) => Record::new(Timestamp::now()),
        Err(err) => {
            // A history that cannot be read starts over: a lower tier for a week is the
            // whole cost.
            tracing::warn!("{err}; starting the availability record over");
            Record::new(Timestamp::now())
        }
    };
    bridge.set_availability(record.tier(Timestamp::now()));

    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tick.tick().await;
    let mut last = (Instant::now(), Timestamp::now());
    loop {
        tick.tick().await;
        let now = (Instant::now(), Timestamp::now());
        let monotonic = now.0 - last.0;
        let wall = now.1.duration_since(last.1);
        let up = credit(monotonic, wall);
        if Duration::try_from(wall).unwrap_or(Duration::ZERO) > monotonic + TICK {
            tracing::debug!("the host slept for about {}s", wall.as_secs());
        }
        record.add(now.1, up);
        if let Err(err) = record.save(&path) {
            tracing::warn!("could not record availability: {err}");
        }
        bridge.set_availability(record.tier(now.1));
        last = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    /// Run `record` up for `minutes`, a tick a minute, from `start`.
    fn up_for(record: &mut Record, start: Timestamp, minutes: i64) -> Timestamp {
        let mut now = start;
        for _ in 0..minutes {
            now += SignedDuration::from_mins(1);
            record.add(now, TICK);
        }
        now
    }

    #[test]
    fn tiers_follow_the_thresholds() {
        assert_eq!(tier_of(1.0), 3);
        assert_eq!(tier_of(0.99), 3);
        assert_eq!(tier_of(0.989), 2);
        assert_eq!(tier_of(0.95), 2);
        assert_eq!(tier_of(0.80), 1);
        assert_eq!(tier_of(0.799), 0);
        assert_eq!(tier_of(0.0), 0);
    }

    #[test]
    fn ticks_add_up_to_minutes_per_day() {
        let start = at("2026-10-11T23:57:00Z");
        let mut r = Record::new(start);
        up_for(&mut r, start, 4);
        let today = jiff::civil::date(2026, 10, 11);
        let tomorrow = jiff::civil::date(2026, 10, 12);
        assert_eq!(r.minutes.get(&today), Some(&2));
        assert_eq!(r.minutes.get(&tomorrow), Some(&2));
    }

    #[test]
    fn partial_ticks_carry_into_whole_minutes() {
        let now = at("2026-10-11T12:00:00Z");
        let mut r = Record::new(now);
        r.add(now, Duration::from_secs(40));
        assert!(r.minutes.is_empty());
        r.add(now, Duration::from_secs(40));
        assert_eq!(r.minutes.values().sum::<u32>(), 1);
        assert_eq!(r.carry, Duration::from_secs(20));
    }

    #[test]
    fn sleep_is_not_credited() {
        // Linux: the monotonic clock stopped, the wall clock ran on for an hour.
        assert_eq!(credit(TICK, SignedDuration::from_hours(1)), TICK);
        // Elsewhere: both ran on, and the tick came an hour late.
        assert_eq!(
            credit(Duration::from_secs(3600), SignedDuration::from_hours(1)),
            TICK * 2
        );
        // The wall clock was stepped back.
        assert_eq!(credit(TICK, SignedDuration::from_secs(-30)), Duration::ZERO);
        assert_eq!(credit(TICK, SignedDuration::from_secs(61)), TICK);
    }

    #[test]
    fn too_little_history_has_no_tier() {
        let start = at("2026-10-11T12:00:00Z");
        let mut r = Record::new(start);
        let now = up_for(&mut r, start, 30);
        assert_eq!(r.tier(now), None);
        let now = up_for(&mut r, now, 30);
        assert!(r.tier(now).is_some());
    }

    #[test]
    fn a_young_history_reports_one_tier_lower() {
        let start = at("2026-10-01T00:00:00Z");
        let mut r = Record::new(start);
        let now = up_for(&mut r, start, 24 * 60);
        assert_eq!(r.tier(now), Some(2), "always up, but only a day of history");
        let now = up_for(&mut r, now, 6 * 24 * 60);
        assert_eq!(r.tier(now), Some(3), "a full week up");
    }

    #[test]
    fn half_the_time_up_is_tier_zero_and_a_young_floor_is_zero() {
        let start = at("2026-10-01T00:00:00Z");
        let mut r = Record::new(start);
        let mut now = start;
        for _ in 0..2 {
            now = up_for(&mut r, now, 12 * 60);
            now += SignedDuration::from_hours(12); // down
        }
        assert_eq!(r.tier(now), Some(0));
    }

    #[test]
    fn the_window_rolls_over() {
        let start = at("2026-10-01T00:00:00Z");
        let mut r = Record::new(start);
        // Up for a week, then down for three days: the window holds four up days of seven.
        let mut now = up_for(&mut r, start, 7 * 24 * 60);
        assert_eq!(r.tier(now), Some(3));
        now += SignedDuration::from_hours(3 * 24);
        let now = up_for(&mut r, now, 1);
        assert_eq!(
            r.minutes.len(),
            5,
            "older days are dropped: {:?}",
            r.minutes
        );
        assert_eq!(r.tier(now), Some(0));
    }

    #[test]
    fn the_record_survives_a_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("availability.toml");
        assert_eq!(Record::load(&path).unwrap(), None);
        let start = at("2026-10-11T12:00:00Z");
        let mut r = Record::new(start);
        up_for(&mut r, start, 90);
        r.carry = Duration::ZERO;
        r.save(&path).unwrap();
        assert_eq!(Record::load(&path).unwrap(), Some(r));
    }

    #[test]
    fn a_record_written_with_chrono_still_reads() {
        // Bridges before the move to jiff wrote `since` as chrono's RFC 3339, in UTC.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("availability.toml");
        std::fs::write(
            &path,
            "since = \"2026-10-11T12:00:00.123456789Z\"\n\n[minutes]\n2026-10-11 = 90\n",
        )
        .unwrap();
        let r = Record::load(&path).unwrap().unwrap();
        assert_eq!(r.since, at("2026-10-11T12:00:00.123456789Z"));
        assert_eq!(r.minutes.get(&jiff::civil::date(2026, 10, 11)), Some(&90));
    }

    #[test]
    fn an_unreadable_record_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("availability.toml");
        std::fs::write(&path, "since = 3").unwrap();
        assert!(Record::load(&path).is_err());
    }
}
