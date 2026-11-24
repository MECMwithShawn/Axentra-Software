//! Expected delivery dates ("Arriving Thursday", "Nov 2 – 5"), in local time.

use chrono::{DateTime, Datelike, Days, Local, NaiveDate, Utc, Weekday};

use super::DateWindow;
use crate::scan::{Prepared, find_word, floor_char};

/// Words a delivery date follows.
const LABELS: &[&str] = &[
    "arriving",
    "arrives",
    "estimated delivery",
    "expected delivery",
    "delivery by",
    "get it by",
    "delivery date",
    "new delivery date",
    "scheduled delivery",
    "scheduled for delivery",
    "expected by",
    "voraussichtliche zustellung",
    "livraison prévue",
    "livraison prevue",
    "entrega estimada",
    "お届け予定日",
    "お届け予定",
    "预计送达",
];

/// Senders that write numeric dates day first.
const DAY_FIRST: &[&str] = &[".co.uk", ".de", ".fr", ".es", ".it", ".nl"];

/// The date must start this close after its label.
const NEAR: usize = 60;
/// Text read after a label (room for the end of a range).
const WINDOW: usize = 100;
/// Labels tried per message at most.
const MAX_LABELS: usize = 50;

/// Before a label in its clause: a promotion's deadline, not a promise
/// about this order ("Order by Dec 18 and get it by Dec 23").
const PROMO_BEFORE: &[&str] = &["order by", "order before", "order within", "order now", "when you order", "if you order", "shop by", "buy by", "buy now"];

/// The expected delivery a message states, resolved against `received`.
/// Anything ambiguous is dropped rather than guessed. Like the status, it
/// is read from the subject, then the head of the body (never a footer or
/// a banner low in the mail).
pub(crate) fn expected(p: &Prepared, received: DateTime<Utc>, sender_domain: &str) -> Option<DateWindow> {
    let today = received.with_timezone(&Local).date_naive();
    let day_first = DAY_FIRST.iter().any(|t| sender_domain.ends_with(t));
    let subject = p.subject.to_ascii_lowercase();
    let head = p.head_lower.get(subject.len()..).unwrap_or("");
    for text in [subject.as_str(), head] {
        let mut labels: Vec<(usize, usize)> = LABELS.iter().flat_map(|l| find_word(text, l).take(MAX_LABELS).map(|i| (i + l.len(), i))).collect();
        labels.sort_unstable();
        labels.dedup_by_key(|(end, _)| *end);
        for (end, start) in labels.into_iter().take(MAX_LABELS) {
            if promotion_before(&text[..start]) {
                continue;
            }
            let window = clause(&text[end..floor_char(text, end + WINDOW)]);
            if let Some(w) = window_in(window, today, day_first) {
                return Some(w);
            }
        }
    }
    None
}

/// The clause before a label (back to the start of its line or sentence)
/// sets a deadline to order by.
fn promotion_before(before: &str) -> bool {
    let near = &before[floor_char(before, before.len().saturating_sub(NEAR))..];
    let near = near.rmatch_indices(['\n', '!', '?', '。']).next().map_or(near, |(i, end)| &near[i + end.len()..]);
    let near = near.rfind(". ").map_or(near, |i| &near[i + 2..]);
    PROMO_BEFORE.iter().any(|w| near.contains(w))
}

/// The text after a label up to the end of its line or sentence: the date
/// must be in the label's own clause, so "3-5 business days.\nOrder date:
/// Oct 18" never reads the order date. A label alone on its line
/// ("Estimated delivery:" above the date) reads the next line.
fn clause(after: &str) -> &str {
    let start = after.find(|c: char| !(c.is_whitespace() || matches!(c, ':' | '：' | '|' | '-'))).unwrap_or(after.len());
    let rest = &after[start..];
    let mut end = rest.find('\n').unwrap_or(rest.len());
    for (i, c) in rest[..end].char_indices() {
        let stop = match c {
            '!' | '?' | '。' => true,
            // "days. Order date" ends a sentence; "22. Oktober" and "Thu.
            // Oct. 22" don't.
            '.' => rest[i + 1..].starts_with(char::is_whitespace) && !abbreviation(&rest[..i]),
            _ => false,
        };
        if stop {
            end = i;
            break;
        }
    }
    &after[..start + end]
}

/// The word before a full stop is a number or a month or weekday name, so
/// the stop doesn't end the sentence.
fn abbreviation(before: &str) -> bool {
    let word = before.rsplit(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
    !word.is_empty() && (word.bytes().all(|b| b.is_ascii_digit()) || month(word).is_some() || weekday(word).is_some())
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(u32, usize),
    Word(String),
    Sym(char),
}

/// Tokens of `s` with their byte offsets: numbers, words (lowercase) and
/// single symbols. 年, 月 and 日 are symbols, so "2026年10月22日" splits.
fn tokens(s: &str) -> Vec<(Tok, usize)> {
    let mut out = Vec::new();
    let mut it = s.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c.is_whitespace() {
            continue;
        }
        let word_char = |c: char| c.is_alphabetic() && !matches!(c, '年' | '月' | '日');
        if c.is_ascii_digit() {
            let mut digits = String::from(c);
            while let Some((_, d)) = it.peek().copied().filter(|(_, d)| d.is_ascii_digit()) {
                digits.push(d);
                it.next();
            }
            out.push((Tok::Num(digits.parse().unwrap_or(u32::MAX), digits.len()), i));
        } else if word_char(c) {
            let mut w = String::from(c);
            while let Some((_, d)) = it.peek().copied().filter(|(_, d)| word_char(*d)) {
                w.push(d);
                it.next();
            }
            out.push((Tok::Word(w.to_lowercase()), i));
        } else {
            out.push((Tok::Sym(c), i));
        }
    }
    out
}

/// A date as written, before it is resolved against the received date.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Written {
    Date(NaiveDate),
    /// "today" (0), "tomorrow" (1).
    DaysAfter(u32),
    Weekday(Weekday),
    Partial { month: u32, day: u32, year: Option<i32>, weekday: Option<Weekday> },
    /// "Thu, 11/5": `a/b` read either way round; the weekday picks the one.
    Either { a: u32, b: u32, year: Option<i32>, weekday: Weekday },
}

impl Written {
    /// The date with the weekday written before it, so a mismatch drops it.
    fn on(self, wd: Weekday) -> Written {
        match self {
            Written::Partial { month, day, year, .. } => Written::Partial { month, day, year, weekday: Some(wd) },
            Written::Date(d) => Written::Partial { month: d.month(), day: d.day(), year: Some(d.year()), weekday: Some(wd) },
            Written::Either { a, b, year, .. } => Written::Either { a, b, year, weekday: wd },
            w => w,
        }
    }
}

/// The first date (or range) in `window` that starts within [`NEAR`] bytes.
/// The first thing shaped like a date decides: if it is ambiguous or
/// impossible, there is no date.
fn window_in(window: &str, today: NaiveDate, day_first: bool) -> Option<DateWindow> {
    let t = tokens(window);
    let mut i = 0;
    while i < t.len() && t[i].1 < NEAR {
        if let Some(found) = date_at(&t, i, day_first) {
            let (first, next) = found?;
            let from = resolve(first, today)?;
            let to = range_end(&t, next, from, day_first, today).filter(|to| *to >= from && (*to - from).num_days() <= 31).unwrap_or(from);
            // A date before the message was sent is not a delivery date
            // (an order date, a past event), unless a range runs into it.
            return (to >= today).then_some(DateWindow { from, to });
        }
        i += 1;
    }
    None
}

/// The end of a range after the first date ("– 5", "to Friday, Oct 23",
/// "and Nov 5"). The end may leave out the month; a weekday alone is the
/// next one on or after the start ("Tuesday – Thursday" received on a
/// Thursday ends two days after the Tuesday, not on the day received).
fn range_end(t: &[(Tok, usize)], next: usize, from: NaiveDate, day_first: bool, today: NaiveDate) -> Option<NaiveDate> {
    let sep = match t.get(next).map(|t| &t.0) {
        Some(Tok::Sym('-' | '–' | '—' | '~' | '〜')) => true,
        Some(Tok::Word(w)) => matches!(w.as_str(), "to" | "and" | "bis" | "und" | "au" | "et" | "a" | "al" | "y"),
        _ => false,
    };
    if !sep {
        return None;
    }
    match date_at(t, next + 1, day_first) {
        Some(Some((end, _))) => {
            return match end {
                Written::Weekday(_) => resolve(end, from),
                _ => resolve(end, today),
            };
        }
        // A date that can't be read without guessing ("10/28 - 11/3"): no
        // end, rather than a bare day.
        Some(None) => return None,
        None => {}
    }
    // "Oct 22 - 9:00 AM": a time of day, not the end of a range.
    if clock_after(t, next + 2) {
        return None;
    }
    // "Oct 30 - 2": a day before the first one is in the next month.
    match t.get(next + 1).map(|t| &t.0) {
        Some(Tok::Num(day, 1 | 2)) if *day >= from.day() => NaiveDate::from_ymd_opt(from.year(), from.month(), *day),
        Some(Tok::Num(day, 1 | 2)) => {
            let (year, month) = if from.month() == 12 { (from.year() + 1, 1) } else { (from.year(), from.month() + 1) };
            NaiveDate::from_ymd_opt(year, month, *day)
        }
        _ => None,
    }
}

/// The date at token `i`: `None` when nothing date-shaped starts there,
/// `Some(None)` when it does but can't be read without guessing, else the
/// date and the token after it.
fn date_at(t: &[(Tok, usize)], i: usize, day_first: bool) -> Option<Option<(Written, usize)>> {
    let tok = |j: usize| t.get(j).map(|t| &t.0);
    let sym = |j: usize, c: char| tok(j) == Some(&Tok::Sym(c));
    match tok(i)? {
        Tok::Word(w) => {
            if let Some(days) = relative(w) {
                return Some(Some((Written::DaysAfter(days), i + 1)));
            }
            if let Some(wd) = weekday(w) {
                let j = if matches!(tok(i + 1), Some(Tok::Sym(',' | '.'))) { i + 2 } else { i + 1 };
                let found = month_day(t, j).or_else(|| day_month(t, j)).or_else(|| iso(t, j)).or_else(|| match tok(j) {
                    Some(Tok::Num(_, 1 | 2)) => numeric(t, j, day_first, true),
                    _ => None,
                });
                if let Some(found) = found {
                    return Some(found.map(|(w, next)| (w.on(wd), next)));
                }
                return Some(Some((Written::Weekday(wd), i + 1)));
            }
            month_day(t, i)
        }
        Tok::Num(_, 4) if sym(i + 1, '-') => iso(t, i),
        Tok::Num(year, 4) if sym(i + 1, '年') => match (tok(i + 2), tok(i + 3), tok(i + 4), tok(i + 5)) {
            (Some(Tok::Num(m, _)), Some(Tok::Sym('月')), Some(Tok::Num(d, _)), Some(Tok::Sym('日'))) => {
                Some(Some((Written::Partial { month: *m, day: *d, year: Some(*year as i32), weekday: None }, i + 6)))
            }
            _ => Some(None),
        },
        Tok::Num(m, 1 | 2) if sym(i + 1, '月') => match (tok(i + 2), tok(i + 3)) {
            (Some(Tok::Num(d, 1 | 2)), Some(Tok::Sym('日'))) => Some(Some((Written::Partial { month: *m, day: *d, year: None, weekday: None }, i + 4))),
            _ => Some(None),
        },
        Tok::Num(_, 1 | 2) => day_month(t, i).or_else(|| numeric(t, i, day_first, false)),
        _ => None,
    }
}

/// ISO "2026-10-22".
fn iso(t: &[(Tok, usize)], i: usize) -> Option<Option<(Written, usize)>> {
    let tok = |j: usize| t.get(j).map(|t| &t.0);
    match (tok(i), tok(i + 1), tok(i + 2), tok(i + 3), tok(i + 4)) {
        (Some(Tok::Num(year, 4)), Some(Tok::Sym('-')), Some(Tok::Num(m, 1 | 2)), Some(Tok::Sym('-')), Some(Tok::Num(d, 1 | 2))) => {
            Some(NaiveDate::from_ymd_opt(*year as i32, *m, *d).map(|date| (Written::Date(date), i + 5)))
        }
        _ => None,
    }
}

/// Days from the received date for "today" and "tomorrow".
fn relative(w: &str) -> Option<u32> {
    match w {
        "today" | "heute" | "aujourd" | "hoy" => Some(0),
        "tomorrow" | "morgen" | "demain" | "mañana" => Some(1),
        _ => None,
    }
}

/// "October 22", "Oct 22nd, 2026".
fn month_day(t: &[(Tok, usize)], i: usize) -> Option<Option<(Written, usize)>> {
    let Some(Tok::Word(w)) = t.get(i).map(|t| &t.0) else { return None };
    let month = month(w)?;
    // "Oct. 22"
    let i = if t.get(i + 1).map(|t| &t.0) == Some(&Tok::Sym('.')) { i + 1 } else { i };
    let Some(Tok::Num(day, 1 | 2)) = t.get(i + 1).map(|t| &t.0) else { return None };
    let mut j = skip_ordinal(t, i + 2);
    let year = year_after(t, &mut j);
    Some(Some((Written::Partial { month, day: *day, year, weekday: None }, j)))
}

/// "22 Oct", "22. Oktober 2026", "22 de octubre".
fn day_month(t: &[(Tok, usize)], i: usize) -> Option<Option<(Written, usize)>> {
    let Some(Tok::Num(day, 1 | 2)) = t.get(i).map(|t| &t.0) else { return None };
    let mut j = skip_ordinal(t, i + 1);
    if t.get(j).map(|t| &t.0) == Some(&Tok::Sym('.')) {
        j += 1;
    }
    if matches!(t.get(j).map(|t| &t.0), Some(Tok::Word(w)) if w == "de") {
        j += 1;
    }
    let Some(Tok::Word(w)) = t.get(j).map(|t| &t.0) else { return None };
    let month = month(w)?;
    j += 1;
    if matches!(t.get(j).map(|t| &t.0), Some(Tok::Word(w)) if w == "de") && matches!(t.get(j + 1).map(|t| &t.0), Some(Tok::Num(_, 4))) {
        j += 1;
    }
    let year = year_after(t, &mut j);
    Some(Some((Written::Partial { month, day: *day, year, weekday: None }, j)))
}

/// `d/m`, `m/d`, `d.m.yyyy`: read only when one part is over 12, or (with
/// a year) when the sender writes day first. After a weekday
/// (`after_weekday`) an ambiguous date is kept both ways round for the
/// weekday to decide. A dotted date is always day first and needs a year
/// or a closing dot ("22.10.", "22.10.2026"): "9.30" is a time.
fn numeric(t: &[(Tok, usize)], i: usize, day_first: bool, after_weekday: bool) -> Option<Option<(Written, usize)>> {
    let tok = |j: usize| t.get(j).map(|t| &t.0);
    let (Some(Tok::Num(a, _)), Some(Tok::Sym(sep @ ('/' | '.'))), Some(Tok::Num(b, 1 | 2))) = (tok(i), tok(i + 1), tok(i + 2)) else { return None };
    let dotted = *sep == '.';
    let mut j = i + 3;
    let mut year = None;
    if tok(j) == Some(&Tok::Sym(*sep))
        && let Some(Tok::Num(y, len @ (2 | 4))) = tok(j + 1)
    {
        year = Some(if *len == 2 { 2000 + *y as i32 } else { *y as i32 });
        j += 2;
    } else if dotted && tok(j) == Some(&Tok::Sym('.')) {
        j += 1;
    } else if dotted || clock_after(t, j) {
        // "9.30am", "10/30 pm": a time of day.
        return None;
    }
    if dotted {
        return Some((*b <= 12).then_some((Written::Partial { month: *b, day: *a, year, weekday: None }, j)));
    }
    let (month, day) = match (*a > 12, *b > 12) {
        (true, false) => (*b, *a),
        (false, true) => (*a, *b),
        (false, false) if year.is_some() && day_first => (*b, *a),
        // The weekday is set by `Written::on`.
        (false, false) if after_weekday => return Some(Some((Written::Either { a: *a, b: *b, year, weekday: Weekday::Mon }, j))),
        _ => return Some(None),
    };
    Some(Some((Written::Partial { month, day, year, weekday: None }, j)))
}

/// Token `j` makes the number before it a time of day: ":30", ".30",
/// "am", "pm", "h", "uhr".
fn clock_after(t: &[(Tok, usize)], j: usize) -> bool {
    match t.get(j).map(|t| &t.0) {
        Some(Tok::Sym(':' | '.')) => matches!(t.get(j + 1).map(|t| &t.0), Some(Tok::Num(_, 2))),
        Some(Tok::Word(w)) => matches!(w.as_str(), "am" | "pm" | "h" | "uhr"),
        _ => false,
    }
}

fn skip_ordinal(t: &[(Tok, usize)], j: usize) -> usize {
    match t.get(j).map(|t| &t.0) {
        Some(Tok::Word(w)) if matches!(w.as_str(), "st" | "nd" | "rd" | "th") => j + 1,
        _ => j,
    }
}

/// A four-digit year at `*j` (after an optional comma), moving `*j` past it.
fn year_after(t: &[(Tok, usize)], j: &mut usize) -> Option<i32> {
    let comma = usize::from(t.get(*j).map(|t| &t.0) == Some(&Tok::Sym(',')));
    match t.get(*j + comma).map(|t| &t.0) {
        Some(Tok::Num(y, 4)) => {
            *j += comma + 1;
            Some(*y as i32)
        }
        _ => None,
    }
}

/// The calendar date `w` means, received on `today`. A year-less date takes
/// the year that puts it within a week before to 120 days after; a weekday
/// that doesn't match the date drops it.
fn resolve(w: Written, today: NaiveDate) -> Option<NaiveDate> {
    match w {
        Written::Date(d) => Some(d),
        Written::Weekday(wd) => {
            let ahead = (7 + wd.num_days_from_monday() - today.weekday().num_days_from_monday()) % 7;
            today.checked_add_days(Days::new(ahead.into()))
        }
        Written::DaysAfter(n) => today.checked_add_days(Days::new(n.into())),
        Written::Partial { month, day, year, weekday } => {
            let date = match year {
                Some(y) => NaiveDate::from_ymd_opt(y, month, day)?,
                None => {
                    let lo = today.checked_sub_days(Days::new(7))?;
                    let hi = today.checked_add_days(Days::new(120))?;
                    [today.year(), today.year() + 1, today.year() - 1]
                        .into_iter()
                        .filter_map(|y| NaiveDate::from_ymd_opt(y, month, day))
                        .find(|d| (lo..=hi).contains(d))?
                }
            };
            weekday.is_none_or(|wd| wd == date.weekday()).then_some(date)
        }
        Written::Either { a, b, year, weekday } => {
            let read = |month, day| resolve(Written::Partial { month, day, year, weekday: Some(weekday) }, today);
            match (read(a, b), read(b, a)) {
                (Some(d), None) | (None, Some(d)) => Some(d),
                (Some(d), Some(e)) if d == e => Some(d),
                _ => None,
            }
        }
    }
}

fn weekday(w: &str) -> Option<Weekday> {
    use Weekday::*;
    Some(match w {
        "monday" | "mon" | "montag" | "lundi" | "lunes" => Mon,
        "tuesday" | "tue" | "tues" | "dienstag" | "mardi" | "martes" => Tue,
        "wednesday" | "wed" | "mittwoch" | "mercredi" | "miércoles" | "miercoles" => Wed,
        "thursday" | "thu" | "thur" | "thurs" | "donnerstag" | "jeudi" | "jueves" => Thu,
        "friday" | "fri" | "freitag" | "vendredi" | "viernes" => Fri,
        "saturday" | "sat" | "samstag" | "sonnabend" | "samedi" | "sábado" | "sabado" => Sat,
        "sunday" | "sun" | "sonntag" | "dimanche" | "domingo" => Sun,
        _ => return None,
    })
}

/// English, German, French and Spanish month names and abbreviations.
fn month(w: &str) -> Option<u32> {
    Some(match w {
        "january" | "jan" | "januar" | "jänner" | "janvier" | "janv" | "enero" | "ene" => 1,
        "february" | "feb" | "februar" | "février" | "fevrier" | "févr" | "fevr" | "febrero" => 2,
        "march" | "mar" | "märz" | "maerz" | "mär" | "mars" | "marzo" => 3,
        "april" | "apr" | "avril" | "avr" | "abril" | "abr" => 4,
        "may" | "mai" | "mayo" => 5,
        "june" | "jun" | "juni" | "juin" | "junio" => 6,
        "july" | "jul" | "juli" | "juillet" | "juil" | "julio" => 7,
        "august" | "aug" | "août" | "aout" | "agosto" | "ago" => 8,
        "september" | "sep" | "sept" | "septembre" | "septiembre" | "setiembre" => 9,
        "october" | "oct" | "oktober" | "okt" | "octobre" | "octubre" => 10,
        "november" | "nov" | "novembre" | "noviembre" => 11,
        "december" | "dec" | "dezember" | "dez" | "décembre" | "decembre" | "déc" | "diciembre" | "dic" => 12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Message;
    use crate::scan::prepare;
    use chrono::TimeZone;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Noon local time on the given day.
    fn at(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Local.with_ymd_and_hms(y, m, d, 12, 0, 0).earliest().unwrap().with_timezone(&Utc)
    }

    fn window(body: &str, received: DateTime<Utc>, domain: &str) -> Option<(NaiveDate, NaiveDate)> {
        let m = Message { subject: "Your order".into(), ..Default::default() };
        expected(&prepare(&m, None, body), received, domain).map(|w| (w.from, w.to))
    }

    fn one(body: &str) -> Option<NaiveDate> {
        // Monday 2026-10-19.
        window(body, at(2026, 10, 19), "shop.example").map(|(from, to)| {
            assert_eq!(from, to, "{body}");
            from
        })
    }

    #[test]
    fn days_weekdays_and_month_names() {
        assert_eq!(at(2026, 10, 19).with_timezone(&Local).weekday(), Weekday::Mon);
        assert_eq!(one("Arriving Thursday"), Some(date(2026, 10, 22)));
        assert_eq!(one("Arriving Monday"), Some(date(2026, 10, 19)));
        assert_eq!(one("Estimated delivery: Thu, Oct 22"), Some(date(2026, 10, 22)));
        assert_eq!(one("Estimated delivery: Thursday, October 22"), Some(date(2026, 10, 22)));
        assert_eq!(one("Delivery date: October 22, 2026"), Some(date(2026, 10, 22)));
        assert_eq!(one("Get it by 22 Oct"), Some(date(2026, 10, 22)));
        assert_eq!(one("Arriving today"), Some(date(2026, 10, 19)));
        assert_eq!(one("Arriving tomorrow by 8pm"), Some(date(2026, 10, 20)));
        assert_eq!(one("Expected delivery 2026-10-22"), Some(date(2026, 10, 22)));
        assert_eq!(one("Voraussichtliche Zustellung: Donnerstag, 22. Oktober"), Some(date(2026, 10, 22)));
        assert_eq!(one("Livraison prévue le jeudi 22 octobre"), Some(date(2026, 10, 22)));
        assert_eq!(one("Entrega estimada: 22 de octubre"), Some(date(2026, 10, 22)));
        assert_eq!(one("Arriving in 3-5 business days"), None);
        assert_eq!(one("Thanks for shopping with us"), None);
    }

    #[test]
    fn year_less_dates_land_near_the_received_date() {
        assert_eq!(window("Arriving Dec 30", at(2026, 12, 28), "shop.example").map(|w| w.0), Some(date(2026, 12, 30)));
        assert_eq!(window("Arriving Jan 2", at(2026, 12, 28), "shop.example").map(|w| w.0), Some(date(2027, 1, 2)));
        // Months ago, and too far ahead: dropped.
        assert_eq!(one("Arriving Jun 2"), None);
    }

    #[test]
    fn ranges() {
        let r = window("Estimated delivery: Nov 2 – 5", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 11, 2), date(2026, 11, 5))));
        let r = window("Arriving between Thursday, Oct 22 and Friday, Oct 23", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 22), date(2026, 10, 23))));
        let r = window("Delivery by Oct 22 - Oct 24", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 22), date(2026, 10, 24))));
        // The end's month is left out and it is in the next month.
        let r = window("Estimated delivery: Oct 30 - 2", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 30), date(2026, 11, 2))));
        let r = window("Estimated delivery: Dec 30 - 2", at(2026, 12, 20), "shop.example");
        assert_eq!(r, Some((date(2026, 12, 30), date(2027, 1, 2))));
        // Weekdays: the end is the one after the start, also when it falls
        // earlier in the week received (Thursday 2026-10-22).
        let r = window("Arriving Tuesday - Thursday", at(2026, 10, 22), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 27), date(2026, 10, 29))));
        let r = window("Arriving Monday - Wednesday", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 19), date(2026, 10, 21))));
    }

    #[test]
    fn numeric_dates_are_read_only_when_unambiguous() {
        let rx = at(2026, 4, 1);
        assert_eq!(window("Delivery date: 03/04/2026", rx, "shop.com"), None);
        assert_eq!(window("Delivery date: 13/04/2026", rx, "shop.com").map(|w| w.0), Some(date(2026, 4, 13)));
        assert_eq!(window("Delivery date: 04/13/2026", rx, "shop.com").map(|w| w.0), Some(date(2026, 4, 13)));
        assert_eq!(window("Delivery date: 03/04/2026", rx, "shop.co.uk").map(|w| w.0), Some(date(2026, 4, 3)));
        assert_eq!(window("Delivery date: 03/04", rx, "shop.co.uk"), None);
    }

    #[test]
    fn japanese_dates_and_conflicting_weekdays() {
        assert_eq!(one("お届け予定日：2026年10月22日"), Some(date(2026, 10, 22)));
        assert_eq!(one("お届け予定：10月22日"), Some(date(2026, 10, 22)));
        // October 23, 2026 is a Friday.
        assert_eq!(one("Arriving Thursday, October 23"), None);
    }

    #[test]
    fn a_weekday_before_a_numeric_date_keeps_the_date() {
        assert_eq!(one("Estimated delivery: Friday 10/30"), Some(date(2026, 10, 30)));
        // 11/5 alone is ambiguous; November 5 is a Thursday, May 11 is not.
        assert_eq!(one("Estimated delivery: Thu, 11/5"), Some(date(2026, 11, 5)));
        assert_eq!(one("Estimated delivery: Thu. 2026-10-29"), Some(date(2026, 10, 29)));
        // A weekday that matches neither reading: dropped, not the next Thursday.
        assert_eq!(one("Estimated delivery: Thu, 11/6"), None);
        assert_eq!(one("Estimated delivery: Friday 10/29"), None);
    }

    #[test]
    fn an_ambiguous_range_end_is_dropped_not_read_as_a_day() {
        let r = window("Estimated delivery: 10/28 - 11/3", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 28), date(2026, 10, 28))));
        let r = window("Estimated delivery: 10/28 - 11/13", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 28), date(2026, 11, 13))));
    }

    #[test]
    fn a_time_of_day_is_not_a_date() {
        // After a range dash: the delivery hours, not the end day.
        let r = window("Estimated delivery: Thursday, October 22 - 9:00 AM - 1:00 PM", at(2026, 10, 19), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 22), date(2026, 10, 22))));
        let r = window("Delivery date: Oct 5 - 9:00 AM", at(2026, 10, 1), "shop.example");
        assert_eq!(r, Some((date(2026, 10, 5), date(2026, 10, 5))));
        assert_eq!(one("Arriving Oct 22 - 9am"), Some(date(2026, 10, 22)));
        // Dotted times (UK "9.30am") are not month.day.
        assert_eq!(window("Delivery by 9.30am tomorrow", at(2026, 8, 10), "shop.co.uk").map(|w| w.0), Some(date(2026, 8, 11)));
        assert_eq!(window("Estimated delivery: 9.30am - 10.30am", at(2026, 8, 10), "shop.co.uk"), None);
        assert_eq!(window("Estimated delivery: 9.30", at(2026, 8, 10), "shop.com"), None);
        assert_eq!(one("Arriving 10:30 tomorrow"), Some(date(2026, 10, 20)));
        // Dotted dates are day first, with a year or a closing dot.
        assert_eq!(window("Voraussichtliche Zustellung: 22.10.", at(2026, 10, 19), "shop.de").map(|w| w.0), Some(date(2026, 10, 22)));
        assert_eq!(window("Delivery date: 03.11.2026", at(2026, 10, 19), "shop.com").map(|w| w.0), Some(date(2026, 11, 3)));
        assert_eq!(window("Delivery date: 22.10.2026", at(2026, 10, 19), "shop.com").map(|w| w.0), Some(date(2026, 10, 22)));
    }

    #[test]
    fn promotions_and_footers_are_not_a_delivery_date() {
        let rx = at(2026, 12, 10);
        // A holiday banner low in the mail.
        let body = format!("Thanks for your order!\n{}\nOrder by Dec 18 and get it by Dec 23", "Lorem ipsum dolor sit amet. ".repeat(200));
        assert_eq!(window(&body, rx, "shop.example"), None);
        // Or in the head: a deadline to order by, not a promise.
        assert_eq!(window("Thanks for your order!\nOrder by Dec 18 and get it by Dec 23.", rx, "shop.example"), None);
        assert_eq!(window("Shop by Dec 18, get it by Dec 23", rx, "shop.example"), None);
        // A promise in its own sentence still counts.
        let r = window("Order by Dec 18 for Christmas. Your order: arriving Dec 15", rx, "shop.example");
        assert_eq!(r, Some((date(2026, 12, 15), date(2026, 12, 15))));
    }

    #[test]
    fn the_date_stays_in_the_labels_sentence() {
        assert_eq!(one("Estimated delivery: 3-5 business days\nOrder date: October 18, 2026"), None);
        assert_eq!(one("Estimated delivery: 3-5 business days. Order date: 10/18/2026"), None);
        // A label alone on its line reads the next line.
        assert_eq!(one("Estimated delivery:\nThursday, October 22"), Some(date(2026, 10, 22)));
        assert_eq!(one("Arriving Thu. Oct. 29. Order date: Oct 18"), Some(date(2026, 10, 29)));
        // A date before the message was sent is not when it arrives.
        assert_eq!(one("Delivery date: October 17, 2026"), None);
        assert_eq!(one("Delivery date: Oct 17"), None);
    }
}
