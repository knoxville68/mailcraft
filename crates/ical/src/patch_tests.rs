// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An event changed here, written into the server's document: only what changed is replaced.
use super::*;
use proptest::prelude::*;

/// A server's document of one event, with what MailCraft doesn't read: an alarm, an attachment,
/// another program's property, a long folded line, and a zone.
const RAW: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Server//EN\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\nTZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:abc-1\r\nDTSTAMP:20261101T080000Z\r\nDTSTART;TZID=Europe/Berlin:20261201T100000\r\nDTEND;TZID=Europe/Berlin:20261201T110000\r\nSUMMARY:Alt\r\nLOCATION:Raum 1\r\nSEQUENCE:3\r\nX-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC\r\nATTACH;FMTTYPE=application/pdf:https://example.org/a-very-long-address-that-is-folded-over\r\n -two-lines.pdf\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Erinnerung\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// What the writer makes of the event as it is now: another title and time, no location, and a
/// zone the server's document doesn't have.
const FRESH: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//MailCraft//MailCraft//EN\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nEND:VTIMEZONE\r\nBEGIN:VTIMEZONE\r\nTZID:Asia/Bangkok\r\nBEGIN:STANDARD\r\nDTSTART:19700101T000000\r\nTZOFFSETFROM:+0700\r\nTZOFFSETTO:+0700\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:abc-1\r\nDTSTAMP:20261122T120000Z\r\nDTSTART;TZID=Asia/Bangkok:20261202T090000\r\nDTEND;TZID=Asia/Bangkok:20261202T100000\r\nSUMMARY:Neu\r\nDESCRIPTION:Mit Notiz\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

#[test]
fn only_what_changed_is_replaced_and_everything_else_stays_as_it_came() {
    let out = patch_event(RAW, FRESH, &[]).unwrap();
    for kept in [
        "X-APPLE-TRAVEL-ADVISORY-BEHAVIOR:AUTOMATIC",
        "ATTACH;FMTTYPE=application/pdf:https://example.org/a-very-long-address-that-is-folded-over\r\n -two-lines.pdf",
        "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Erinnerung\r\nTRIGGER:-PT15M\r\nEND:VALARM",
        "PRODID:-//Example//Server//EN",
        "UID:abc-1",
    ] {
        assert!(out.contains(kept), "{kept} kept:\n{out}");
    }
    assert!(out.contains("SUMMARY:Neu") && !out.contains("SUMMARY:Alt"));
    assert!(out.contains("DTSTART;TZID=Asia/Bangkok:20261202T090000") && !out.contains("20261201T100000"));
    assert!(out.contains("DESCRIPTION:Mit Notiz"), "the event's own description, not the alarm's");
    assert!(!out.contains("LOCATION:"), "a location taken away is gone");
    assert!(out.contains("SEQUENCE:4") && !out.contains("SEQUENCE:3"));
    assert!(out.contains("DTSTAMP:20261122T120000Z") && !out.contains("DTSTAMP:20261101"));
    // The zone it now names is added once, before the event; the one it had stays as it was.
    assert_eq!(out.matches("TZID:Asia/Bangkok").count(), 1);
    assert_eq!(out.matches("BEGIN:VTIMEZONE").count(), 2);
    assert!(out.contains("TZOFFSETFROM:+0200"), "the server's own zone, not the writer's empty one");
    assert!(out.find("TZID:Asia/Bangkok") < out.find("BEGIN:VEVENT"));
    // And it reads back as the changed event.
    let events = crate::parse(&out, &|_| 0).events;
    assert_eq!(events.len(), 1);
    assert_eq!(events.first().map(|e| e.title.as_str()), Some("Neu"));
}

#[test]
fn what_else_changed_is_taken_too_and_new_alarms_replace_the_old() {
    let fresh = FRESH.replace(
        "END:VEVENT",
        "CATEGORIES:Arbeit\r\nCLASS:PRIVATE\r\nRRULE:FREQ=WEEKLY\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT30M\r\nEND:VALARM\r\nEND:VEVENT",
    );
    let raw = RAW.replace("SEQUENCE:3", "SEQUENCE:3\r\nCATEGORIES:Alt\r\nRRULE:FREQ=DAILY");
    let out = patch_event(&raw, &fresh, &["CATEGORIES", "CLASS", "RRULE", "VALARM"]).unwrap();
    for (now, gone) in [("CATEGORIES:Arbeit", "CATEGORIES:Alt"), ("RRULE:FREQ=WEEKLY", "RRULE:FREQ=DAILY"), ("TRIGGER:-PT30M", "TRIGGER:-PT15M")] {
        assert!(out.contains(now) && !out.contains(gone), "{now} instead of {gone}:\n{out}");
    }
    assert!(out.contains("CLASS:PRIVATE"));
    assert_eq!(out.matches("BEGIN:VALARM").count(), 1);
    assert_eq!(out.matches("END:VALARM").count(), 1);
    // Not named: what the server has stays.
    let out = patch_event(&raw, &fresh, &[]).unwrap();
    assert!(out.contains("CATEGORIES:Alt") && out.contains("TRIGGER:-PT15M") && !out.contains("CLASS:"));
    // Taken away: the alarm is gone.
    let out = patch_event(&raw, FRESH, &["VALARM"]).unwrap();
    assert!(!out.contains("VALARM"), "{out}");
}

#[test]
fn a_document_out_of_order_or_without_one_main_event_is_not_changed() {
    let two_mains = RAW.replace("END:VEVENT\r\nEND:VCALENDAR", "END:VEVENT\r\nBEGIN:VEVENT\r\nUID:other\r\nEND:VEVENT\r\nEND:VCALENDAR");
    assert!(patch_event(&two_mains, FRESH, &[]).is_none());
    assert!(patch_event("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nEND:VCALENDAR\r\n", FRESH, &[]).is_none());
    assert!(patch_event(" folded first\r\n", FRESH, &[]).is_none());
    assert!(patch_event(RAW, "", &[]).is_none());
}

/// A series by Berlin's clock with its second day changed on its own.
const SERIES: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:s-1\r\nDTSTART;TZID=Europe/Berlin:20261123T100000\r\nDTEND;TZID=Europe/Berlin:20261123T110000\r\nRRULE:FREQ=DAILY;COUNT=5\r\nSUMMARY:Serie\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:s-1\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261124T100000\r\nDTSTART;TZID=Europe/Berlin:20261124T120000\r\nDTEND;TZID=Europe/Berlin:20261124T130000\r\nSUMMARY:Verschoben\r\nX-KEEP:yes\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn one(summary: &str, day: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:ignored#x\r\nDTSTART;TZID=Europe/Berlin:{day}T150000\r\nDTEND;TZID=Europe/Berlin:{day}T160000\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

#[test]
fn the_series_a_day_changed_already_and_a_new_day_are_each_changed_in_their_own_event() {
    let (day2, day3) = (one("Später", "20261124"), one("Neu", "20261125"));
    let patches = [
        Patch { target: Target::Main, fresh: &FRESH.replace("Neu", "Serie neu"), also: &[] },
        Patch { target: Target::Changed("20261124T100000"), fresh: &day2, also: &[] },
        Patch { target: Target::New("20261125"), fresh: &day3, also: &[] },
    ];
    let out = patch_resource(SERIES, &patches, &[]).unwrap();
    assert!(out.contains("RRULE:FREQ=DAILY;COUNT=5"), "the rule stays:\n{out}");
    assert!(out.contains("SUMMARY:Serie neu") && out.contains("SUMMARY:Später") && !out.contains("Verschoben"), "{out}");
    assert!(out.contains("X-KEEP:yes"), "what the changed day had stays");
    // The new day is named as the series names its days, by the series' uid.
    assert!(out.contains("UID:s-1\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261125T100000\r\nDTSTART;TZID=Europe/Berlin:20261125T150000"), "{out}");
    assert!(!out.contains("ignored#x"));
    assert_eq!(out.matches("BEGIN:VEVENT").count(), 3);
    let events = crate::parse(&out, &|_| 0).events;
    assert_eq!(events.len(), 3);
    // A day that is there already isn't new, and one that isn't there can't be changed.
    assert!(patch_resource(SERIES, &[Patch { target: Target::New("20261124"), fresh: &day2, also: &[] }], &[]).is_none());
    assert!(patch_resource(SERIES, &[Patch { target: Target::Changed("20261126T100000"), fresh: &day2, also: &[] }], &[]).is_none());
    assert!(patch_resource(SERIES, &[Patch { target: Target::New("2026-11-25"), fresh: &day3, also: &[] }], &[]).is_none());
}

#[test]
fn a_day_changed_on_its_own_is_taken_out_and_the_series_stays() {
    let out = patch_resource(SERIES, &[], &["20261124T100000"]).unwrap();
    assert!(!out.contains("Verschoben") && out.contains("SUMMARY:Serie"));
    assert_eq!(out.matches("BEGIN:VEVENT").count(), 1);
    assert_eq!(out.matches("END:VEVENT").count(), 1);
}

proptest! {
    #[test]
    fn any_text_never_panics(raw in ".{0,400}", fresh in ".{0,400}") {
        let _ = patch_event(&raw, &fresh, &[]);
        let _ = patch_event(&raw, FRESH, &[]);
        let _ = patch_event(RAW, &fresh, &[]);
        let _ = patch_resource(SERIES, &[Patch { target: Target::New("20261125"), fresh: &fresh, also: &["VALARM"] }], &[&raw]);
    }
}
