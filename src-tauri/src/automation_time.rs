//! Shared due-time math for the local automation engines (free build).
//!
//! Both engines — cookie-bot warming slots and agent schedules — answer the
//! same three questions: *when does this fire next*, *has this slot already
//! fired*, and *is a timezone name real*. One implementation, so the two
//! surfaces cannot drift apart on what "Tuesday 09:30 in Europe/Berlin"
//! means.
//!
//! All functions are pure — no clock, no store — so the math is unit-testable
//! against fixed instants, including the DST boundaries that make naive
//! "minutes after midnight" arithmetic wrong.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

/// A weekly firing slot: days it fires on plus the minute of the day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WeeklySlot {
  /// bit0 = Monday … bit6 = Sunday, as the wire format counts.
  pub days_mask: u8,
  /// Minutes after midnight in `timezone`.
  pub run_at_minute: u32,
}

/// Whether `timezone` names a zone the scheduler understands.
pub(crate) fn valid_timezone(name: &str) -> bool {
  name.parse::<chrono_tz::Tz>().is_ok()
}

/// The zone named by `name`, or UTC for an empty/unknown name. The store
/// validates on save, so this only degrades for a hand-edited file.
pub(crate) fn zone(name: &str) -> chrono_tz::Tz {
  name.parse().unwrap_or(chrono_tz::Tz::UTC)
}

/// The mask bit for a weekday, Monday-first as the wire format counts
/// (chrono numbers Monday as 1 … Sunday as 7; the mask counts Mon=1 … Sun=64).
fn mask_bit(weekday_from_monday: u32) -> u8 {
  debug_assert!((1..=7).contains(&weekday_from_monday));
  1u8 << (weekday_from_monday - 1)
}

/// The earliest instant strictly after `now` at which `slot` fires in `tz`.
///
/// Looks ahead at most eight local days — a week plus slack for a zone whose
/// DST shift pushes the slot past midnight — which covers every weekly mask.
/// `None` means the mask selects no weekday at all.
pub(crate) fn next_weekly_fire(
  slot: WeeklySlot,
  tz_name: &str,
  now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
  if slot.days_mask == 0 {
    return None;
  }
  let tz = zone(tz_name);
  let today = now.with_timezone(&tz).date_naive();
  for day_offset in 0i64..8 {
    let date = today
      .checked_add_signed(Duration::days(day_offset))
      .expect("date arithmetic within a week is safe");
    let weekday_from_monday = date.weekday().number_from_monday();
    if slot.days_mask & mask_bit(weekday_from_monday) == 0 {
      continue;
    }
    let minute = slot.run_at_minute.min(24 * 60 - 1);
    let fire_local = date
      .and_hms_opt(0, 0, 0)
      .map(|midnight_local| midnight_local + Duration::minutes(minute as i64))
      .expect("valid calendar date and bounded minute");
    let fire_utc = tz
      .from_local_datetime(&fire_local)
      .single()
      .or_else(|| tz.from_local_datetime(&fire_local).earliest())
      .map(|dt| dt.with_timezone(&Utc));
    if let Some(fire_utc) = fire_utc {
      if fire_utc > now {
        return Some(fire_utc);
      }
    }
  }
  None
}

/// The local instant of `slot` on today's date in `tz_name`, when today's
/// weekday is selected and the slot's time has passed within the last
/// `window_minutes`. `None` means: not today, still ahead, or long missed —
/// a slot far in the past is skipped, not queued.
pub(crate) fn due_today(
  slot: WeeklySlot,
  tz_name: &str,
  now: DateTime<Utc>,
  window_minutes: i64,
) -> Option<DateTime<Utc>> {
  let tz = zone(tz_name);
  let today = now.with_timezone(&tz).date_naive();
  let weekday = today.weekday().number_from_monday();
  if slot.days_mask & (1u8 << (weekday - 1)) == 0 {
    return None;
  }
  let minute = slot.run_at_minute.min(24 * 60 - 1);
  let local = today
    .and_hms_opt(0, 0, 0)
    .map(|midnight| midnight + Duration::minutes(minute as i64))?;
  let utc = tz
    .from_local_datetime(&local)
    .single()
    .or_else(|| tz.from_local_datetime(&local).earliest())
    .map(|dt| dt.with_timezone(&Utc))?;
  let age = now.signed_duration_since(utc);
  if age >= Duration::zero() && age <= Duration::minutes(window_minutes) {
    Some(utc)
  } else {
    None
  }
}

/// A stable identity for one firing of one slot on one local date, so a
/// restart can never double-fire the same slot. Two slots that share a date
/// and minute collide deliberately: only one of them can own the key.
pub(crate) fn weekly_fire_key(
  scope: &str,
  slot: WeeklySlot,
  tz_name: &str,
  fire: DateTime<Utc>,
) -> String {
  let local = fire.with_timezone(&zone(tz_name));
  format!(
    "{scope}:w:{}:{}",
    local.date_naive(),
    slot.run_at_minute.min(24 * 60 - 1)
  )
}

/// Parse an RFC3339 instant as the wire sends it (`runOnceAt`).
pub(crate) fn parse_instant(text: &str) -> Option<DateTime<Utc>> {
  DateTime::parse_from_rfc3339(text)
    .ok()
    .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// 2026-10-25 is a Sunday; 09:00 UTC.
  fn sunday_morning() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 25, 9, 0, 0).unwrap()
  }

  #[test]
  fn timezone_names_are_validated() {
    assert!(valid_timezone("Europe/Berlin"));
    assert!(valid_timezone("UTC"));
    assert!(!valid_timezone("Mars/Olympus"));
    assert!(!valid_timezone(""));
  }

  #[test]
  fn the_next_fire_skips_disabled_weekdays() {
    // Monday bit only (0b0000001). From Sunday 2026-10-25 the next Monday is
    // 2026-10-26, at 09:30 Berlin — which is 08:30 UTC after the October DST
    // change (CET, UTC+1).
    let slot = WeeklySlot {
      days_mask: 0b0000_0001,
      run_at_minute: 9 * 60 + 30,
    };
    let fire = next_weekly_fire(slot, "Europe/Berlin", sunday_morning()).unwrap();
    assert_eq!(fire, Utc.with_ymd_and_hms(2026, 10, 26, 8, 30, 0).unwrap());
  }

  #[test]
  fn a_slot_later_today_still_fires_today() {
    // Sunday bit (0b1000000), 23:00 Berlin on Sunday 2026-10-25 (CET, +1).
    let slot = WeeklySlot {
      days_mask: 0b0100_0000,
      run_at_minute: 23 * 60,
    };
    let fire = next_weekly_fire(slot, "Europe/Berlin", sunday_morning()).unwrap();
    assert_eq!(fire, Utc.with_ymd_and_hms(2026, 10, 25, 22, 0, 0).unwrap());
  }

  #[test]
  fn a_slot_already_past_today_moves_to_next_week() {
    // Sunday 08:00 Berlin has already passed at 09:00 UTC (10:00 Berlin).
    let slot = WeeklySlot {
      days_mask: 0b0100_0000,
      run_at_minute: 8 * 60,
    };
    let fire = next_weekly_fire(slot, "Europe/Berlin", sunday_morning()).unwrap();
    assert_eq!(fire, Utc.with_ymd_and_hms(2026, 11, 1, 7, 0, 0).unwrap());
  }

  #[test]
  fn an_empty_mask_never_fires() {
    let slot = WeeklySlot {
      days_mask: 0,
      run_at_minute: 600,
    };
    assert!(next_weekly_fire(slot, "UTC", sunday_morning()).is_none());
  }

  #[test]
  fn every_weekday_mask_fires_within_eight_days() {
    for bit in 0..7u8 {
      let slot = WeeklySlot {
        days_mask: 1 << bit,
        run_at_minute: 7 * 60 + 15,
      };
      let fire = next_weekly_fire(slot, "Europe/Berlin", sunday_morning())
        .unwrap_or_else(|| panic!("mask bit {bit} found no slot in eight days"));
      assert!(fire > sunday_morning());
    }
  }

  #[test]
  fn fire_keys_differ_per_date_and_minute() {
    let slot = WeeklySlot {
      days_mask: 127,
      run_at_minute: 600,
    };
    let a = Utc.with_ymd_and_hms(2026, 10, 26, 5, 0, 0).unwrap();
    let b = a + Duration::days(1);
    assert_ne!(
      weekly_fire_key("cb:p1", slot, "UTC", a),
      weekly_fire_key("cb:p1", slot, "UTC", b)
    );
    let later = WeeklySlot {
      days_mask: 127,
      run_at_minute: 700,
    };
    assert_ne!(
      weekly_fire_key("cb:p1", slot, "UTC", a),
      weekly_fire_key("cb:p1", later, "UTC", a)
    );
  }

  #[test]
  fn instants_round_trip_from_rfc3339() {
    let parsed = parse_instant("2026-10-26T08:30:00Z").unwrap();
    assert_eq!(
      parsed,
      Utc.with_ymd_and_hms(2026, 10, 26, 8, 30, 0).unwrap()
    );
    assert!(parse_instant("not a date").is_none());
  }
}
