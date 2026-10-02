//! Calendar from a private iCal (ICS) link: Google "Secret address in iCal format" or Outlook "Publish
//! calendar → ICS". No account sign-in, nothing leaves the PC except the download of your own calendar.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Event {
    pub title: String,
    pub start: i64, // unix ms
    pub end: i64,
    pub all_day: bool,
    /// People invited (organizer first), for naming speakers in call notes.
    pub attendees: Vec<String>,
}

pub fn fetch(url: &str) -> Result<String, String> {
    if !(url.starts_with("https://") || url.starts_with("webcal://")) {
        return Err("Use the https:// (or webcal://) iCal link from your calendar's settings".into());
    }
    let url = url.replacen("webcal://", "https://", 1);
    ureq::get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|e| format!("Couldn't download the calendar ({})", e.kind()))?
        .into_string()
        .map_err(|e| e.to_string())
}

/// Lines with RFC 5545 folding undone (a line starting with space/tab continues the previous one).
fn unfold(ics: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in ics.split('\n') {
        let line = raw.trim_end_matches('\r');
        match (line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')), out.last_mut()) {
            (Some(rest), Some(prev)) => prev.push_str(rest),
            _ => out.push(line.to_string()),
        }
    }
    out
}

/// Parses DTSTART/DTEND values. "Z" = UTC; a TZID or no zone = local time.
/// ponytail: TZID is treated as the PC's local zone; fine for your own calendar, off for other zones.
fn parse_time(value: &str) -> Option<(DateTime<Local>, bool)> {
    if value.len() == 8 {
        let d = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest()?, true));
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let t = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((Utc.from_utc_datetime(&t).with_timezone(&Local), false));
    }
    let t = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Some((Local.from_local_datetime(&t).earliest()?, false))
}

fn weekday(code: &str) -> Option<Weekday> {
    Some(match code.trim_start_matches(|c: char| c == '+' || c == '-' || c.is_ascii_digit()) {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

#[derive(Default)]
struct Raw {
    title: String,
    start: Option<(DateTime<Local>, bool)>,
    end: Option<DateTime<Local>>,
    rrule: Option<String>,
    exdates: Vec<i64>,
    attendees: Vec<String>,
}

/// "ATTENDEE;CN=Ahmed Ali;ROLE=…" + "mailto:ahmed@x.com" → "Ahmed Ali" (or "ahmed" when there's no CN).
fn person(key: &str, value: &str) -> Option<String> {
    let cn = key.split(';').find_map(|p| p.strip_prefix("CN=")).map(|n| n.trim_matches('"').trim().to_string());
    let name = cn.filter(|n| !n.is_empty() && !n.contains('@')).or_else(|| {
        let mail = value.trim_start_matches("mailto:").trim_start_matches("MAILTO:");
        mail.split('@').next().filter(|s| !s.is_empty()).map(|s| s.replace(['.', '_'], " "))
    })?;
    Some(name)
}

/// Occurrences of `ev` that overlap [from, to). Supports DAILY/WEEKLY with INTERVAL, BYDAY, UNTIL, COUNT, EXDATE.
fn occurrences(ev: &Raw, from: DateTime<Local>, to: DateTime<Local>) -> Vec<Event> {
    let Some((start, all_day)) = ev.start else { return vec![] };
    let length = ev.end.map(|e| e - start).unwrap_or(if all_day { Duration::days(1) } else { Duration::hours(1) });
    let make = |s: DateTime<Local>| Event {
        title: if ev.title.is_empty() { "Busy".into() } else { ev.title.clone() },
        start: s.timestamp_millis(),
        end: (s + length).timestamp_millis(),
        all_day,
        attendees: ev.attendees.clone(),
    };
    let Some(rule) = &ev.rrule else {
        return if start < to && start + length > from { vec![make(start)] } else { vec![] };
    };
    let part = |k: &str| rule.split(';').find_map(|p| p.strip_prefix(&format!("{k}=")).map(str::to_string));
    let freq = part("FREQ").unwrap_or_default();
    let interval = part("INTERVAL").and_then(|v| v.parse::<i64>().ok()).unwrap_or(1).max(1);
    let until = part("UNTIL").and_then(|u| parse_time(&u)).map(|(t, _)| t);
    let count = part("COUNT").and_then(|v| v.parse::<usize>().ok());
    let days: Vec<Weekday> = part("BYDAY").map(|v| v.split(',').filter_map(weekday).collect()).unwrap_or_default();
    if freq != "DAILY" && freq != "WEEKLY" {
        return if start < to && start + length > from { vec![make(start)] } else { vec![] }; // unsupported: first only
    }
    let (mut out, mut emitted, mut day) = (Vec::new(), 0usize, start);
    while day < to && until.is_none_or(|u| day <= u) && count.is_none_or(|c| emitted < c) {
        let weeks = (day.date_naive() - start.date_naive()).num_days().div_euclid(7);
        let hit = match freq.as_str() {
            "DAILY" => (day.date_naive() - start.date_naive()).num_days() % interval == 0,
            _ => weeks % interval == 0 && (if days.is_empty() { day.weekday() == start.weekday() } else { days.contains(&day.weekday()) }),
        };
        if hit {
            emitted += 1;
            if day + length > from && !ev.exdates.contains(&day.timestamp_millis()) {
                out.push(make(day));
            }
        }
        day += Duration::days(1);
        if out.len() > 500 {
            break;
        }
    }
    out
}

/// Events overlapping the next `days` days (from the start of today), sorted by start.
pub fn upcoming(ics: &str, days: i64) -> Vec<Event> {
    let from = Local.from_local_datetime(&Local::now().date_naive().and_hms_opt(0, 0, 0).unwrap()).earliest().unwrap();
    let to = from + Duration::days(days);
    let (mut events, mut cur): (Vec<Event>, Option<Raw>) = (Vec::new(), None);
    for line in unfold(ics) {
        if line == "BEGIN:VEVENT" {
            cur = Some(Raw::default());
            continue;
        }
        if line == "END:VEVENT" {
            if let Some(ev) = cur.take() {
                events.extend(occurrences(&ev, from, to));
            }
            continue;
        }
        let (Some(ev), Some((key, value))) = (cur.as_mut(), line.split_once(':')) else { continue };
        let name = key.split(';').next().unwrap_or(key);
        match name {
            "SUMMARY" => ev.title = value.replace("\\,", ",").replace("\\;", ";").replace("\\n", " "),
            "DTSTART" => ev.start = parse_time(value),
            "DTEND" => ev.end = parse_time(value).map(|(t, _)| t),
            "RRULE" => ev.rrule = Some(value.to_string()),
            "EXDATE" => ev.exdates.extend(value.split(',').filter_map(parse_time).map(|(t, _)| t.timestamp_millis())),
            "ORGANIZER" | "ATTENDEE" => {
                if let Some(p) = person(key, value).filter(|p| !ev.attendees.contains(p)) {
                    ev.attendees.push(p);
                }
            }
            _ => {}
        }
    }
    events.sort_by_key(|e| e.start);
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ics(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\n{body}END:VCALENDAR\r\n")
    }

    fn today_at(h: u32) -> String {
        Local::now().date_naive().and_hms_opt(h, 0, 0).unwrap().format("%Y%m%dT%H%M%S").to_string()
    }

    #[test]
    fn single_event_today_with_folding() {
        let cal = ics(&format!(
            "BEGIN:VEVENT\r\nSUMMARY:Design re\r\n view\\, v2\r\nDTSTART:{}\r\nDTEND:{}\r\nEND:VEVENT\r\n",
            today_at(15),
            today_at(16)
        ));
        let ev = upcoming(&cal, 1);
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].title, "Design review, v2");
        assert_eq!(ev[0].end - ev[0].start, 3_600_000);
    }

    #[test]
    fn weekly_rule_expands_and_respects_exdate() {
        let start = Local::now().date_naive() - Duration::days(14);
        let first = start.and_hms_opt(10, 0, 0).unwrap().format("%Y%m%dT%H%M%S").to_string();
        let skipped = (start + Duration::days(21)).and_hms_opt(10, 0, 0).unwrap().format("%Y%m%dT%H%M%S").to_string();
        let cal = ics(&format!(
            "BEGIN:VEVENT\r\nSUMMARY:Standup\r\nDTSTART:{first}\r\nRRULE:FREQ=WEEKLY\r\nEXDATE:{skipped}\r\nEND:VEVENT\r\n"
        ));
        let ev = upcoming(&cal, 28); // today .. +4 weeks: occurrences at +0 and +14 days from today (+7 excluded)
        let offsets: Vec<i64> = ev.iter().map(|e| (e.start - ev[0].start) / 86_400_000).collect();
        assert_eq!(offsets, [0, 14, 21], "{ev:?}");
    }

    #[test]
    fn utc_and_all_day() {
        let (t, all_day) = parse_time("20261001T120000Z").unwrap();
        assert!(!all_day && t.with_timezone(&Utc).format("%H:%M").to_string() == "12:00");
        assert!(parse_time("20261001").unwrap().1);
        assert!(fetch("http://insecure.example/cal.ics").is_err());
    }
}
