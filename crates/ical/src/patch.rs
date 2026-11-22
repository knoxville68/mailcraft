// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An event changed here, written into the document a calendar server sent (#209 D6, ADR 0026):
//! only the properties MailCraft changes are replaced; every other line of the document (alarms,
//! attachments, another program's own properties, the zones it names) is kept as it came, folded
//! as it came (RFC 5545 section 3.1).

/// The properties of the event that are written anew: what the owner can change from here, and
/// the stamps of a change.
const REPLACED: [&str; 8] = ["SUMMARY", "LOCATION", "DESCRIPTION", "DTSTART", "DTEND", "DURATION", "DTSTAMP", "LAST-MODIFIED"];

/// Most lines a document may have to be changed here (a document is at most a megabyte).
const MAX_LINES: usize = 100_000;

/// One content line with the lines folded into it, as they came, and its name in upper case.
struct Group<'a> {
    lines: Vec<&'a str>,
    name: String,
}

fn groups(doc: &str) -> Option<Vec<Group<'_>>> {
    let mut out: Vec<Group<'_>> = Vec::new();
    for (n, line) in doc.split('\n').enumerate() {
        if n >= MAX_LINES {
            return None;
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with([' ', '\t']) {
            // (A folded line goes with the one before; one with none before is no document.)
            out.last_mut()?.lines.push(line);
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let name = line.split([':', ';']).next().unwrap_or_default().trim().to_ascii_uppercase();
        out.push(Group { lines: vec![line], name });
    }
    Some(out)
}

/// The value of a group's first line after its name and parameters (`BEGIN:VEVENT` → `VEVENT`).
fn value<'a>(g: &Group<'a>) -> &'a str {
    g.lines.first().and_then(|l| l.split_once(':')).map(|(_, v)| v.trim()).unwrap_or_default()
}

/// The components a document's groups are in, for each group: the path of `BEGIN`s it is under.
fn paths(groups: &[Group<'_>]) -> Option<Vec<Vec<String>>> {
    let mut stack: Vec<String> = Vec::new();
    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        match g.name.as_str() {
            "BEGIN" => {
                out.push(stack.clone());
                if stack.len() >= 16 {
                    return None;
                }
                stack.push(value(g).to_ascii_uppercase());
            }
            "END" => {
                let ends = value(g).to_ascii_uppercase();
                if stack.last() != Some(&ends) {
                    return None;
                }
                stack.pop();
                out.push(stack.clone());
            }
            _ => out.push(stack.clone()),
        }
    }
    stack.is_empty().then_some(out)
}

fn is_event(path: &[String]) -> bool {
    path.len() == 2 && path.first().is_some_and(|c| c == "VCALENDAR") && path.get(1).is_some_and(|c| c == "VEVENT")
}

/// Whether a group is a line of one of the event's alarms, or begins or ends one.
fn in_alarm(g: &Group<'_>, path: &[String]) -> bool {
    path.get(2).is_some_and(|c| c == "VALARM")
        || (is_event(path) && matches!(g.name.as_str(), "BEGIN" | "END") && value(g).eq_ignore_ascii_case("VALARM"))
}

fn events(groups: &[Group<'_>]) -> usize {
    groups.iter().filter(|g| g.name == "BEGIN" && value(g).eq_ignore_ascii_case("VEVENT")).count()
}

/// A document's zones: each one's `TZID` and the groups of its block.
fn zones(groups: &[Group<'_>], paths: &[Vec<String>]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < groups.len() {
        let top = paths.get(at).is_some_and(|p| p.len() == 1);
        if top && groups.get(at).is_some_and(|g| g.name == "BEGIN" && value(g).eq_ignore_ascii_case("VTIMEZONE")) {
            let start = at;
            let mut tzid = String::new();
            while at < groups.len() {
                let Some(g) = groups.get(at) else { break };
                if g.name == "TZID" && paths.get(at).is_some_and(|p| p.len() == 2) {
                    tzid = value(g).to_string();
                }
                at += 1;
                if g.name == "END" && value(g).eq_ignore_ascii_case("VTIMEZONE") && paths.get(at - 1).is_some_and(|p| p.len() == 1) {
                    break;
                }
            }
            out.push((tzid, start..at));
            continue;
        }
        at += 1;
    }
    out
}

/// Which event of a server's document a [`Patch`] changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target<'a> {
    /// The event, or the series: the one event without a `RECURRENCE-ID`.
    Main,
    /// A day of the series changed on its own already: the event with this `RECURRENCE-ID` value.
    Changed(&'a str),
    /// A day of the series changed on its own from now on, by its day as the series counts it
    /// (`YYYYMMDD`): a new event for it, named as the series names its days.
    New(&'a str),
}

/// One change to a server's document: the event `target` names becomes what `fresh` (a document
/// [`crate::write_in`] made of one event as it is now) says of it.
#[derive(Clone, Copy, Debug)]
pub struct Patch<'a> {
    pub target: Target<'a>,
    pub fresh: &'a str,
    /// More of the event's properties to take from `fresh` than [`REPLACED`]; `VALARM` takes its
    /// alarms, the old ones all dropped.
    pub also: &'a [&'a str],
}

/// `raw` (the server's document of one event) with the event's [`REPLACED`] properties taken from
/// `fresh`, its `SEQUENCE` one higher, and the zones `fresh` names that `raw` lacks added;
/// everything else as it came. `None` when either is not a whole calendar, or `fresh` is not one
/// event, or `raw` has no one event without a `RECURRENCE-ID`.
pub fn patch_event(raw: &str, fresh: &str, also: &[&str]) -> Option<String> {
    patch_resource(raw, &[Patch { target: Target::Main, fresh, also }], &[])
}

/// `raw` with each patch made in turn, and the days of the series changed on their own that
/// `drop` names (by `RECURRENCE-ID` value) taken out. `None` when any of it can't be done: a
/// document that isn't whole, an event that isn't there (or a new day that is), a patch that isn't
/// of one event.
pub fn patch_resource(raw: &str, patches: &[Patch<'_>], drop: &[&str]) -> Option<String> {
    let mut doc = raw.to_string();
    // (New days first: they are named by the series as the server has it.)
    let (new, others): (Vec<&Patch<'_>>, Vec<&Patch<'_>>) = patches.iter().take(64).partition(|p| matches!(p.target, Target::New(_)));
    for patch in new.into_iter().chain(others) {
        doc = patch_one(&doc, patch)?;
    }
    if !drop.is_empty() {
        doc = drop_days(&doc, drop)?;
    }
    Some(doc)
}

/// The events of a document: where each begins and ends (its `END`, inclusive), and its
/// `RECURRENCE-ID` value when it has one.
fn vevents(groups: &[Group<'_>], paths: &[Vec<String>]) -> Vec<(usize, usize, Option<String>)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, Option<String>)> = None;
    for (at, (g, path)) in groups.iter().zip(paths).enumerate() {
        let top = path.len() == 1;
        if top && g.name == "BEGIN" && value(g).eq_ignore_ascii_case("VEVENT") {
            open = Some((at, None));
        } else if top && g.name == "END" && value(g).eq_ignore_ascii_case("VEVENT") {
            if let Some((begin, rid)) = open.take() {
                out.push((begin, at, rid));
            }
        } else if g.name == "RECURRENCE-ID"
            && is_event(path)
            && let Some((_, rid)) = open.as_mut()
        {
            *rid = Some(value(g).to_string());
        }
    }
    out
}

/// A content line as one line again (RFC 5545 section 3.1).
fn unfolded(g: &Group<'_>) -> String {
    let mut out = String::new();
    for (n, line) in g.lines.iter().enumerate() {
        out.push_str(if n == 0 { line } else { line.get(1..).unwrap_or_default() });
    }
    out
}

fn patch_one(raw: &str, patch: &Patch<'_>) -> Option<String> {
    let (old, new) = (groups(raw)?, groups(patch.fresh)?);
    let (old_paths, new_paths) = (paths(&old)?, paths(&new)?);
    if events(&new) != 1 {
        return None;
    }
    let found = vevents(&old, &old_paths);
    let mains: Vec<&(usize, usize, Option<String>)> = found.iter().filter(|(_, _, rid)| rid.is_none()).collect();
    let [main] = mains.as_slice() else { return None };
    let alarms = patch.also.contains(&"VALARM");
    let wanted = |g: &Group<'_>, p: &[String]| {
        (alarms && in_alarm(g, p)) || (is_event(p) && (REPLACED.contains(&g.name.as_str()) || patch.also.contains(&g.name.as_str())))
    };
    let have: Vec<String> = zones(&old, &old_paths).into_iter().map(|(tzid, _)| tzid).collect();
    let missing: Vec<std::ops::Range<usize>> =
        zones(&new, &new_paths).into_iter().filter(|(tzid, _)| !tzid.is_empty() && !have.contains(tzid)).map(|(_, r)| r).collect();
    let first_event = found.iter().map(|(begin, _, _)| *begin).min()?;
    let mut out = String::with_capacity(raw.len().saturating_add(patch.fresh.len()));
    let mut push = |line: &str| {
        out.push_str(line);
        out.push_str("\r\n");
    };
    let zones_in = |push: &mut dyn FnMut(&str)| {
        for range in &missing {
            for line in new.get(range.clone()).into_iter().flatten().flat_map(|g| g.lines.iter()) {
                push(line);
            }
        }
    };
    match patch.target {
        Target::New(day) => {
            if day.len() != 8 || !day.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            // Named as the series names its days: its start, on that day.
            let in_main = |name: &str| {
                old.get(main.0..=main.1)?.iter().zip(old_paths.get(main.0..=main.1)?).find(|(g, p)| is_event(p) && g.name == name).map(|(g, _)| g)
            };
            let start = unfolded(in_main("DTSTART")?);
            let (params, at) = start.split_once(':')?;
            let rid = format!("RECURRENCE-ID{}:{day}{}", params.get("DTSTART".len()..)?, at.trim().get(8..)?);
            if found.iter().any(|(_, _, r)| r.as_deref() == rid.split_once(':').map(|(_, v)| v)) {
                return None;
            }
            let uid = in_main("UID")?;
            let skip = ["UID", "RECURRENCE-ID", "RRULE", "RDATE", "EXDATE", "SEQUENCE"];
            for (at, g) in old.iter().enumerate() {
                if at == first_event {
                    zones_in(&mut push);
                }
                let closes = g.name == "END" && value(g).eq_ignore_ascii_case("VCALENDAR") && old_paths.get(at).is_some_and(|p| p.is_empty());
                if closes {
                    push("BEGIN:VEVENT");
                    uid.lines.iter().for_each(|l| push(l));
                    push(&rid);
                    for (g, p) in new.iter().zip(&new_paths) {
                        let inside = is_event(p) || p.len() > 2;
                        if inside && !(is_event(p) && skip.contains(&g.name.as_str())) {
                            g.lines.iter().for_each(|l| push(l));
                        }
                    }
                    push("END:VEVENT");
                }
                g.lines.iter().for_each(|l| push(l));
            }
        }
        Target::Main | Target::Changed(_) => {
            let range = match patch.target {
                Target::Changed(rid) => found.iter().find(|(_, _, r)| r.as_deref() == Some(rid.trim()))?,
                _ => *main,
            };
            let (begin, end) = (range.0, range.1);
            let taken: Vec<&Group<'_>> = new.iter().zip(&new_paths).filter(|(g, p)| wanted(g, p)).map(|(g, _)| g).collect();
            let sequence = old
                .get(begin..=end)?
                .iter()
                .zip(old_paths.get(begin..=end)?)
                .find(|(g, p)| is_event(p) && g.name == "SEQUENCE")
                .and_then(|(g, _)| value(g).parse::<u32>().ok())
                .map_or(1, |n| n.saturating_add(1));
            for (at, (g, path)) in old.iter().zip(&old_paths).enumerate() {
                if at == first_event {
                    zones_in(&mut push);
                }
                let ours = (begin..=end).contains(&at);
                if ours && at == end {
                    for line in taken.iter().flat_map(|g| g.lines.iter()) {
                        push(line);
                    }
                    push(&format!("SEQUENCE:{sequence}"));
                }
                let replaced = ours && (wanted(g, path) || (is_event(path) && g.name == "SEQUENCE"));
                if !replaced {
                    g.lines.iter().for_each(|l| push(l));
                }
            }
        }
    }
    Some(out)
}

/// `raw` without the days of its series changed on their own that `drop` names.
fn drop_days(raw: &str, drop: &[&str]) -> Option<String> {
    let old = groups(raw)?;
    let found = vevents(&old, &paths(&old)?);
    let gone: Vec<(usize, usize)> =
        found.iter().filter(|(_, _, rid)| rid.as_deref().is_some_and(|r| drop.iter().any(|d| d.trim() == r))).map(|(b, e, _)| (*b, *e)).collect();
    let mut out = String::with_capacity(raw.len());
    for (at, g) in old.iter().enumerate() {
        if !gone.iter().any(|(b, e)| (*b..=*e).contains(&at)) {
            for line in &g.lines {
                out.push_str(line);
                out.push_str("\r\n");
            }
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "patch_tests.rs"]
mod tests;
