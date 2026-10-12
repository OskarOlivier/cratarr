//! Working out what is playing. Jellyfin's session record can be stale with some clients (observed: it kept an
//! old track as "paused at 0 s" while the activity log correctly said a new track had started), so the
//! activity log is the primary source: the newest audio playback that has not been stopped.

use crate::jellyfin::ActivityEntry;

const START: &str = "AudioPlayback";
const STOP: &str = "AudioPlaybackStopped";

/// The item id of the track that is currently playing according to the activity log, if any.
/// `None` also means "the log says everything has stopped"; callers decide whether to fall back.
pub fn current_from_activity(entries: &[ActivityEntry], now_epoch: i64, max_age_secs: i64) -> Option<String> {
    let mut starts: Vec<&ActivityEntry> = entries.iter().filter(|e| e.kind == START && e.item_id.is_some()).collect();
    starts.sort_by(|a, b| b.date.cmp(&a.date)); // ISO-8601 UTC strings sort chronologically
    let current = starts.into_iter().find(|s| {
        !entries
            .iter()
            .any(|e| e.kind == STOP && e.item_id == s.item_id && e.date >= s.date)
    })?;
    let age = now_epoch - parse_iso_epoch(&current.date)?;
    (age <= max_age_secs).then(|| current.item_id.clone()).flatten()
}

/// True when the log has any audio playback entries at all (so its verdict, even "stopped", is meaningful).
pub fn has_audio_history(entries: &[ActivityEntry]) -> bool {
    entries.iter().any(|e| e.kind == START || e.kind == STOP)
}

/// Seconds since the Unix epoch for `YYYY-MM-DDTHH:MM:SS[.fraction]Z`; `None` if it does not parse.
pub fn parse_iso_epoch(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // days from civil (Howard Hinnant's algorithm)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOG: &str = include_str!("../tests/fixtures/activity.json");

    fn entries() -> Vec<ActivityEntry> {
        let page: crate::jellyfin::ItemsPage<ActivityEntry> = serde_json::from_str(LOG).unwrap();
        page.items
    }

    fn e(date: &str, kind: &str, item: &str) -> ActivityEntry {
        ActivityEntry { date: date.into(), kind: kind.into(), item_id: Some(item.into()) }
    }

    #[test]
    fn iso_dates_parse_to_epoch_seconds() {
        assert_eq!(parse_iso_epoch("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_epoch("2000-03-01T00:00:00.1234567Z"), Some(951_868_800));
        assert_eq!(parse_iso_epoch("2026-10-12T13:28:00.5Z"), Some(1_791_811_680));
        assert_eq!(parse_iso_epoch("not a date"), None);
        assert_eq!(parse_iso_epoch("2026-13-12T13:28:00Z"), None);
    }

    #[test]
    fn newest_unstopped_start_is_the_current_track() {
        let now = parse_iso_epoch("2026-10-12T13:29:00Z").unwrap();
        assert_eq!(current_from_activity(&entries(), now, 3 * 3600).as_deref(), Some("item-e"));
    }

    #[test]
    fn a_start_and_a_stop_of_other_items_in_the_same_second_do_not_confuse_it() {
        let log = vec![
            e("2026-10-12T13:21:00.9Z", "AudioPlayback", "new"),
            e("2026-10-12T13:21:00.9Z", "AudioPlaybackStopped", "old"),
            e("2026-10-12T13:12:58.1Z", "AudioPlayback", "old"),
        ];
        let now = parse_iso_epoch("2026-10-12T13:22:00Z").unwrap();
        assert_eq!(current_from_activity(&log, now, 3600).as_deref(), Some("new"));
    }

    #[test]
    fn a_stopped_track_is_not_current() {
        let log = vec![e("2026-10-12T13:22:16Z", "AudioPlaybackStopped", "a"), e("2026-10-12T13:21:00Z", "AudioPlayback", "a")];
        let now = parse_iso_epoch("2026-10-12T13:23:00Z").unwrap();
        assert_eq!(current_from_activity(&log, now, 3600), None);
    }

    #[test]
    fn very_old_starts_are_ignored() {
        let log = vec![e("2026-10-12T08:00:00Z", "AudioPlayback", "a")];
        let now = parse_iso_epoch("2026-10-12T13:00:00Z").unwrap();
        assert_eq!(current_from_activity(&log, now, 3 * 3600), None);
    }

    #[test]
    fn other_activity_types_are_ignored() {
        let log = vec![e("2026-10-12T13:28:00Z", "VideoPlayback", "v"), e("2026-10-12T13:00:00Z", "AudioPlayback", "a")];
        let now = parse_iso_epoch("2026-10-12T13:29:00Z").unwrap();
        assert_eq!(current_from_activity(&log, now, 3600).as_deref(), Some("a"));
        assert!(has_audio_history(&log));
        assert!(!has_audio_history(&[e("2026-10-12T13:28:00Z", "SessionStarted", "x")]));
    }
}
