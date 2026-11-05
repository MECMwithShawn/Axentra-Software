//! The rules that decide a message's topic, in order: the first that
//! decides wins.

use super::lexicon::{AUTOMATED_LOCAL_PARTS, BOOKING_CODE_LABELS, SIGNALS, Signals};
use super::{Assignment, Input, Reason, Source, Topic, TopicRule, rule_matches};
use crate::junk::analyze::{Severity, registrable};
use crate::junk::domain_of;
use crate::orders::ExtractionKind;
use crate::orders::carriers::carrier_sender;
use crate::orders::scam_signs;
use crate::scan::{Prepared, contains_word, find_word, floor_char, has_unsubscribe, local_part_is, prepare};

/// Scored signals need at least this, and this much more than the next topic.
const MIN_SCORE: f32 = 0.55;
const MIN_MARGIN: f32 = 0.15;
/// Room for rounding when comparing sums of scores.
const SLACK: f32 = 1e-4;
/// More recipients than this isn't a conversation with someone you know.
const MAX_PERSONAL_RECIPIENTS: usize = 20;
/// Places one phrase is looked at per text at most.
const MAX_PHRASE_HITS: usize = 50;

#[cfg(test)]
thread_local! {
    /// Makes [`classify`] panic, to test [`classify_guarded`].
    pub(super) static PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The topic of one message. Never panics on any input (see
/// [`classify_guarded`]).
pub fn classify(input: &Input) -> Assignment {
    #[cfg(test)]
    if PANIC.with(|p| p.get()) {
        panic!("classify: test panic");
    }
    let m = input.message;
    let decided = |topic: Topic, confidence: f32, source: Source, reasons: Vec<Reason>| Assignment { topic, confidence, source, reasons };
    let danger = input.junk == Some(Severity::Danger);

    // 1. The user's choice for this message.
    if let Some(topic) = input.message_override {
        return decided(topic, 1.0, Source::MessageOverride, vec![Reason::MessageOverride]);
    }
    // 2. The user's choice for the sender (it beats everything, even warning signs).
    let address = m.from.address.trim().to_ascii_lowercase();
    if let Some(rule) = sender_rule(input.rules, m.account_id, &address) {
        let (source, mut reasons) = match rule.is_domain {
            true => (Source::DomainRule, vec![Reason::DomainRule { pattern: rule.pattern.clone() }]),
            false => (Source::SenderRule, vec![Reason::SenderRule { pattern: rule.pattern.clone() }]),
        };
        if danger {
            reasons.push(Reason::Suspicious);
        }
        return decided(rule.topic, 1.0, source, reasons);
    }
    // 3. Phishing never lands in Orders, Finance or Receipts. Mail order
    // detection hasn't looked at (switched off, too old, a sender not
    // tracked) goes through its scam gates here.
    let p = prepare(m, input.html, input.text);
    let suppressed = match &input.order {
        Some(o) => o.kind == ExtractionKind::Suppressed,
        None => scam_signs(m, &p, input.sender_authenticated).is_some(),
    };
    if danger || suppressed {
        return decided(Topic::Other, 1.0, Source::Suspicious, vec![Reason::Suspicious]);
    }
    let a = automatic(input, &p, &address);
    // A sender posing as someone else (a shop's name on another domain)
    // never gets the topics of an order, a bill or a receipt, even when
    // the check only says Caution: that is the scam.
    if input.junk == Some(Severity::Caution) && input.impersonation && matches!(a.topic, Topic::Orders | Topic::Finance | Topic::Receipts) {
        return decided(Topic::Other, 1.0, Source::Suspicious, vec![Reason::Suspicious]);
    }
    a
}

/// Steps 4 to 7 of [`classify`]: what the message itself says.
fn automatic(input: &Input, p: &Prepared, address: &str) -> Assignment {
    let m = input.message;
    let decided = |topic: Topic, confidence: f32, source: Source, reasons: Vec<Reason>| Assignment { topic, confidence, source, reasons };
    // 4. Meetings.
    if m.meeting_kind.is_some() {
        return decided(Topic::Events, 0.95, Source::Meeting, vec![Reason::Meeting]);
    }
    // 5. Order emails.
    if let Some(o) = input.order.as_ref().filter(|o| o.kind == ExtractionKind::Order) {
        return decided(Topic::Orders, o.confidence.clamp(0.0, 1.0), Source::Order, vec![Reason::OrderEmail { merchant: o.merchant.to_string() }]);
    }
    // 6. Someone you know, writing to you.
    let local = address.split('@').next().unwrap_or("");
    let known = input.contact_name.is_some() || input.replied_before;
    if known
        && !AUTOMATED_LOCAL_PARTS.iter().any(|w| local_part_is(local, w))
        && !has_unsubscribe(p)
        && m.to.len() + m.cc.len() <= MAX_PERSONAL_RECIPIENTS
    {
        let name = input.contact_name.unwrap_or(&m.from.name).trim().to_string();
        return decided(Topic::People, 0.9, Source::Person, vec![Reason::KnownPerson { name }]);
    }
    // 7. Scored signals.
    signals(input, p, address)
}

/// [`classify`], with a failure sorted as Other instead of stopping the caller.
pub fn classify_guarded(input: &Input) -> Assignment {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| classify(input))).unwrap_or_else(|_| fallback())
}

fn fallback() -> Assignment {
    Assignment { topic: Topic::Other, confidence: 0.0, source: Source::Fallback, reasons: vec![Reason::LowConfidence { best: Topic::Other }] }
}

/// The rule for `address`: the exact address (this account's before all
/// accounts'), then the most specific domain (this account's first).
fn sender_rule<'a>(rules: &'a [TopicRule], account: i64, address: &str) -> Option<&'a TopicRule> {
    let mine = |r: &&TopicRule| r.account_key != 0;
    let exact = rules.iter().filter(|r| !r.is_domain && rule_matches(r, account, address)).max_by_key(mine);
    exact.or_else(|| rules.iter().filter(|r| r.is_domain && rule_matches(r, account, address)).max_by_key(|r| (r.pattern.split('.').count(), mine(r))))
}

/// One topic's score and why.
struct Score {
    topic: Topic,
    value: f32,
    reasons: Vec<Reason>,
}

/// Topics whose mail can carry an unsubscribe footer and still be what
/// they are.
const TRANSACTIONAL: [Topic; 5] = [Topic::Orders, Topic::Receipts, Topic::Finance, Topic::Travel, Topic::Notifications];

fn signals(input: &Input, p: &Prepared, address: &str) -> Assignment {
    let subject = p.subject.to_ascii_lowercase();
    // The body part of the head (the subject is scored on its own).
    let head = p.head_lower.get(subject.len()..).unwrap_or("");
    let local = address.split('@').next().unwrap_or("");
    let domain = registrable(domain_of(address));
    let name = domain.split('.').next().unwrap_or("");
    let unsubscribe = has_unsubscribe(p);

    // The Orders words stand in for order detection only when it hasn't run.
    let used = |sig: &&Signals| sig.topic != Topic::Orders || input.order.is_none();
    let hits = phrase_hits(SIGNALS.iter().filter(used), &subject, head);
    let mut scores: Vec<Score> = Vec::new();
    for sig in SIGNALS.iter().filter(used) {
        let mut s = Score { topic: sig.topic, value: 0.0, reasons: Vec::new() };
        let (subject_hits, head_hits) = word_hits(sig, &hits, &mut s.reasons);
        s.value += (0.35 * subject_hits as f32).min(0.7) + (0.15 * head_hits as f32).min(0.45);
        // An invitation in the subject that the body repeats.
        if sig.topic == Topic::Events && subject_hits > 0 && head_hits > 0 {
            s.value += 0.1;
        }
        let phrase = subject_hits + head_hits > 0;
        if let Some(part) = sig.local_parts.iter().find(|w| local_part_is(local, w)) {
            // Social: a "notify" address alone is any app's.
            if sig.topic != Topic::Social || phrase {
                s.value += 0.2;
                s.reasons.push(Reason::SenderLocalPart { part: part.to_string() });
            }
        }
        if sig.domain_hints.iter().any(|h| hinted(name, h)) || sig.domains.contains(&domain.as_str()) {
            s.value += 0.2;
        }
        structure(sig.topic, input, p, &domain, local, unsubscribe, &mut s);
        s.value = s.value.min(1.0);
        scores.push(s);
    }
    decide(scores)
}

/// A domain name (`name`, the registrable domain's first label) says
/// `hint`: one of its parts (split at `-` and digits) is the hint, or, for a
/// hint of four letters or more, starts or ends with it ("bankofexample",
/// "britishairways"). Never anywhere inside: "air" is not in "airtable" or
/// "repair", "trip" not in "stripe".
fn hinted(name: &str, hint: &str) -> bool {
    name.split(|c: char| c == '-' || c.is_ascii_digit())
        .any(|part| part == hint || (hint.len() >= 4 && (part.starts_with(hint) || part.ends_with(hint))))
}

/// One topic phrase found in the subject or the head.
struct Hit {
    topic: Topic,
    word: &'static str,
    in_subject: bool,
    start: usize,
    end: usize,
}

/// Every phrase of `signals` in the subject and the head, except one that
/// lies inside another topic's longer phrase found at the same place: the
/// "reminder" of "event reminder" counts for Events only, the "seat" of
/// "save your seat" not for Travel.
fn phrase_hits<'a>(signals: impl Iterator<Item = &'a Signals>, subject: &str, head: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for sig in signals {
        for w in sig.words {
            for (text, in_subject) in [(subject, true), (head, false)] {
                for start in find_word(text, w).take(MAX_PHRASE_HITS) {
                    hits.push(Hit { topic: sig.topic, word: w, in_subject, start, end: start + w.len() });
                }
            }
        }
    }
    let inside = |h: &Hit| {
        hits.iter().any(|o| o.topic != h.topic && o.in_subject == h.in_subject && o.start <= h.start && o.end >= h.end && o.end - o.start > h.end - h.start)
    };
    let keep: Vec<bool> = hits.iter().map(|h| !inside(h)).collect();
    hits.into_iter().zip(keep).filter_map(|(h, keep)| keep.then_some(h)).collect()
}

/// Subject and head hits of a topic's words (each word once per text), with
/// a reason for each.
fn word_hits(sig: &Signals, hits: &[Hit], reasons: &mut Vec<Reason>) -> (usize, usize) {
    let (mut in_subject, mut in_head) = (0, 0);
    for w in sig.words {
        let found = |subject: bool| hits.iter().any(|h| h.topic == sig.topic && h.word == *w && h.in_subject == subject);
        if found(true) {
            in_subject += 1;
            reasons.push(Reason::SubjectWord { word: w.to_string() });
        }
        if found(false) {
            in_head += 1;
            reasons.push(Reason::BodyPhrase { phrase: w.to_string() });
        }
    }
    (in_subject, in_head)
}

/// The structure signals of `topic`.
fn structure(topic: Topic, input: &Input, p: &Prepared, domain: &str, local: &str, unsubscribe: bool, s: &mut Score) {
    let head = &p.head_lower;
    match topic {
        Topic::Orders => {
            if carrier_sender(domain).is_some() {
                s.value += 0.3;
                s.reasons.push(Reason::CarrierSender);
            }
        }
        Topic::Receipts => {
            // An amount right after "total" or "invoice".
            if ["total", "invoice"].iter().any(|w| find_word(head, w).any(|i| money(&head[i..floor_char(head, i + 40)]))) {
                s.value += 0.2;
            }
        }
        Topic::Finance => {
            if local_part_is(local, "alerts") && (domain.contains("bank") || contains_word(head, "bank") || contains_word(head, "banking")) {
                s.value += 0.2;
                s.reasons.push(Reason::SenderLocalPart { part: "alerts".into() });
            }
        }
        Topic::Travel => {
            if BOOKING_CODE_LABELS.iter().any(|l| find_word(head, l).any(|i| booking_code(&p.head[i + l.len()..floor_char(&p.head, i + l.len() + 20)]))) {
                s.value += 0.2;
                s.reasons.push(Reason::BodyPhrase { phrase: "booking code".into() });
            }
        }
        Topic::Events => {
            let ics = input.attachments.iter().any(|a| a.name.to_ascii_lowercase().ends_with(".ics") || a.content_type.to_ascii_lowercase().starts_with("text/calendar"));
            if ics && input.message.meeting_kind.is_none() {
                s.value += 0.4;
            }
        }
        Topic::Notifications => {
            if find_word(head, "code").any(|i| code_near(head, i)) {
                s.value += 0.3;
                s.reasons.push(Reason::VerificationCode);
            }
        }
        Topic::Newsletters => {
            if unsubscribe {
                s.value += 0.45;
                s.reasons.push(Reason::UnsubscribeLink);
            }
            if p.links.len() > 15 {
                s.value += 0.15;
            }
        }
        Topic::People | Topic::Social | Topic::Other => {}
    }
}

/// The best topic when it is sure enough, else Other.
fn decide(mut scores: Vec<Score>) -> Assignment {
    // A receipt with an unsubscribe footer stays a receipt.
    let transactional = scores.iter().any(|s| TRANSACTIONAL.contains(&s.topic) && s.value >= MIN_SCORE);
    if transactional {
        scores.retain(|s| s.topic != Topic::Newsletters);
    }
    // On a tie Receipts beats Finance, and Finance beats Notifications.
    let preference = |t: Topic| match t {
        Topic::Receipts => 2,
        Topic::Finance => 1,
        _ => 0,
    };
    let best = scores.iter().enumerate().max_by(|(_, a), (_, b)| {
        a.value.total_cmp(&b.value).then(preference(a.topic).cmp(&preference(b.topic))).then(b.topic.cmp(&a.topic))
    });
    let Some((at, &Score { topic, value, .. })) = best else { return fallback() };
    let beaten_on_tie = |t: Topic| match topic {
        Topic::Receipts => matches!(t, Topic::Finance | Topic::Notifications),
        Topic::Finance => t == Topic::Notifications,
        _ => false,
    };
    let tied = |v: f32| (value - v).abs() < SLACK;
    let second = scores.iter().filter(|s| s.topic != topic && !(beaten_on_tie(s.topic) && tied(s.value))).map(|s| s.value).fold(0.0, f32::max);
    if value >= MIN_SCORE - SLACK && value - second >= MIN_MARGIN - SLACK {
        let best = scores.swap_remove(at);
        return Assignment { topic, confidence: value, source: Source::Signals, reasons: best.reasons };
    }
    let closest = if value > 0.0 { topic } else { Topic::Other };
    Assignment { topic: Topic::Other, confidence: 0.0, source: Source::Fallback, reasons: vec![Reason::LowConfidence { best: closest }] }
}

/// `$`, `€`, `£` or `¥` next to a number.
fn money(s: &str) -> bool {
    ["$", "€", "£", "¥"].iter().any(|sign| {
        s.match_indices(sign).any(|(i, _)| {
            s[i + sign.len()..].trim_start_matches(' ').starts_with(|c: char| c.is_ascii_digit())
                || s[..i].trim_end_matches(' ').ends_with(|c: char| c.is_ascii_digit())
        })
    })
}

/// A 4–8 digit code within 20 bytes of "code" at `i`: a word of digits
/// alone, not the year in a promo code ("FALL2026").
fn code_near(head: &str, i: usize) -> bool {
    let b = head.as_bytes();
    let mut from = floor_char(head, i.saturating_sub(20));
    let mut to = floor_char(head, i + "code".len() + 20);
    // Whole words only: widen the window over a word it cuts.
    while from > 0 && b[from - 1].is_ascii_alphanumeric() && i - from < 40 {
        from -= 1;
    }
    while to < b.len() && b[to].is_ascii_alphanumeric() && to - i < 60 {
        to += 1;
    }
    b[from..to].split(|c| !c.is_ascii_alphanumeric()).any(|w| (4..=8).contains(&w.len()) && w.iter().all(u8::is_ascii_digit))
}

/// Six letters and digits (with at least one letter) after a booking label.
fn booking_code(after: &str) -> bool {
    let rest = after.trim_start_matches([' ', ':', '#', '\u{a0}']);
    let code: String = rest.chars().take_while(char::is_ascii_alphanumeric).collect();
    code.len() == 6 && code.bytes().any(|b| b.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::super::OrderHint;
    use super::*;
    use crate::model::{Attachment, EmailAddress, MeetingMessageKind, Message};

    /// A message and what is known about it.
    #[derive(Clone, Default)]
    struct Mail {
        message: Message,
        html: Option<String>,
        text: String,
        attachments: Vec<Attachment>,
        order: Option<(ExtractionKind, f32)>,
        junk: Option<Severity>,
        impersonation: bool,
        authenticated: bool,
        contact: Option<String>,
        replied: bool,
        rules: Vec<TopicRule>,
        message_override: Option<Topic>,
    }

    fn mail(name: &str, address: &str, subject: &str) -> Mail {
        Mail { message: Message { account_id: 1, from: EmailAddress::new(name, address), subject: subject.into(), ..Default::default() }, ..Default::default() }
    }

    impl Mail {
        fn html(mut self, html: &str) -> Self {
            self.text = crate::html::html_to_text(html);
            self.html = Some(html.into());
            self
        }

        fn text(mut self, text: &str) -> Self {
            self.text = text.into();
            self
        }

        fn run(&self) -> Assignment {
            classify(&self.input())
        }

        fn input(&self) -> Input<'_> {
            Input {
                message: &self.message,
                html: self.html.as_deref(),
                text: &self.text,
                attachments: &self.attachments,
                order: self.order.map(|(kind, confidence)| OrderHint { kind, confidence, merchant: "Brightleaf Home" }),
                junk: self.junk,
                impersonation: self.impersonation,
                sender_authenticated: self.authenticated,
                contact_name: self.contact.as_deref(),
                replied_before: self.replied,
                rules: &self.rules,
                message_override: self.message_override,
            }
        }

        fn topic(&self) -> Topic {
            let a = self.run();
            assert!(!a.reasons.is_empty(), "{a:?}");
            a.topic
        }
    }

    fn newsletter(body: &str) -> String {
        format!("<h2>{body}</h2><p>{body}</p><p><a href=\"https://news.example/read\">Read online</a> · <a href=\"https://news.example/unsubscribe\">Unsubscribe</a></p>")
    }

    fn para(lines: &[&str]) -> String {
        lines.iter().map(|l| format!("<p>{l}</p>")).collect()
    }

    #[test]
    fn a_promo_code_with_a_year_is_no_verification_code() {
        let links: String = (0..20).map(|i| format!("<a href=\"https://northpeak.example/p/{i}\">Item {i}</a> ")).collect();
        let html = format!(
            "<p>Log in to your account and use code FALL2026 at checkout.</p><p>{links}</p><p><a href=\"https://northpeak.example/unsubscribe\">Unsubscribe</a></p>"
        );
        let promo = Mail { order: Some((ExtractionKind::None, 0.2)), ..mail("Northpeak", "noreply@northpeak.example", "48 hours only: 30% off everything").html(&html) };
        let a = promo.run();
        assert!(!a.reasons.contains(&Reason::VerificationCode), "{a:?}");
        assert_eq!(a.topic, Topic::Newsletters, "{a:?}");
        for code in ["BF2026", "SAVE1234", "A12345", "2026X"] {
            assert!(!code_near(&format!("use code {code} today"), 4), "{code}");
        }
        for text in ["use code 4829 today", "code: 482913.", "your code is 12345678"] {
            assert!(code_near(text, text.find("code").unwrap()), "{text}");
        }
    }

    #[test]
    fn one_message_per_topic() {
        let mut person = mail("Dana Whitfield", "dana@home.example", "Pickup tomorrow?").text("My flight lands at 6, can you grab me?");
        person.contact = Some("Dana Whitfield".into());
        assert_eq!(person.topic(), Topic::People);
        assert_eq!(person.run().reasons, [Reason::KnownPerson { name: "Dana Whitfield".into() }]);

        let mut order = mail("Brightleaf Home", "orders@brightleaf.example", "Your order has shipped");
        order.order = Some((ExtractionKind::Order, 0.9));
        assert_eq!(order.run(), Assignment { topic: Topic::Orders, confidence: 0.9, source: Source::Order, reasons: vec![Reason::OrderEmail { merchant: "Brightleaf Home".into() }] });

        let receipt = mail("Skyline Air", "no-reply@skyline.example", "Receipt: Seattle flight").html(&para(&["Total charged: $412.60", "Thank you for your purchase."]));
        assert_eq!(receipt.topic(), Topic::Receipts);
        let invoice = mail("Lumen Studio", "hello@lumen.example", "Invoice INV-2048").html(&para(&["Invoice INV-2048 for $12,400.00 is attached. Due in 30 days."]));
        assert_eq!(invoice.topic(), Topic::Receipts);
        let bank = mail("Harbor Bank", "alerts@harborbank.example", "Your monthly statement is ready").html(&para(&["Your October statement is available in online banking."]));
        assert_eq!(bank.topic(), Topic::Finance);
        let trip = mail("Skyline Air", "no-reply@skyline.example", "Your trip to Seattle is confirmed")
            .html(&para(&["Confirmation code: <b>K7Q2XP</b>", "Mon, Oct 26 · SFO 8:05 AM → SEA 10:12 AM", "Seat 14C · Main Cabin"]));
        assert_eq!(trip.topic(), Topic::Travel);
        let reminder = mail("Meetup", "events@meetup.example", "Event reminder: Design meetup tomorrow").text("Event reminder: the design meetup starts at 6pm.");
        assert_eq!(reminder.topic(), Topic::Events);
        let unknown = mail("Design Guild", "guild@designers.example", "Event reminder: Design meetup tomorrow").text("Event reminder: the design meetup starts at 6pm.");
        assert_eq!(unknown.topic(), Topic::Events, "a subject phrase the body repeats");
        assert!(!unknown.run().reasons.contains(&Reason::SubjectWord { word: "reminder".into() }));
        let webinar = mail("Corvid Cloud", "hello@corvid.example", "Save your seat: webinar on Thursday").text("Join our product webinar on Thursday at 10am.");
        assert_eq!(webinar.topic(), Topic::Events);
        let invite = mail("Lakeside Makers", "events@lakeside-makers.example", "You\u{2019}re invited: Autumn maker night – RSVP").text("Join us Thursday at 7pm. RSVP by Wednesday.");
        assert_eq!(invite.topic(), Topic::Events);
        let social = mail("Circlet", "notify@circlet.example", "Jordan commented on your photo").text("Jordan Lee commented on your photo: 'Great shot!'");
        assert_eq!(social.topic(), Topic::Social);
        let code = mail("Corvid Cloud", "no-reply@corvid.example", "Your verification code is 482913").text("Use code 482913 to sign in. It expires in 10 minutes.");
        assert_eq!(code.topic(), Topic::Notifications);
        assert!(code.run().reasons.contains(&Reason::VerificationCode));
        let news = mail("Northwind Weekly", "newsletter@northwind.example", "This week at Northwind: supply chain wins").html(&newsletter("Here's what's new this week."));
        assert_eq!(news.topic(), Topic::Newsletters);
        let promo = mail("Northpeak Outfitters", "news@northpeak.example", "30% off new arrivals – order now, ships free").html(&newsletter("New arrivals"));
        assert_eq!(Mail { order: Some((ExtractionKind::None, 0.2)), ..promo }.topic(), Topic::Newsletters);
        let other = mail("Sam", "sam@random.example", "Quick question").text("Do you have a minute later?");
        assert_eq!(other.run(), fallback());

        let mut ics = mail("Lakeside Makers", "hello@lakeside-makers.example", "Invitation: Autumn maker night").text("Add it to your calendar.");
        assert_eq!(ics.run().topic, Topic::Other, "not sure without the invite");
        ics.attachments.push(Attachment { name: "invite.ics".into(), content_type: "text/calendar".into(), ..Default::default() });
        assert_eq!(ics.topic(), Topic::Events);
    }

    #[test]
    fn seats_and_names_that_contain_a_hint_are_not_travel() {
        for (address, subject) in [
            ("team@airtable.com", "A seat was added to your workspace"),
            ("feedback@slack.com", "Your Slack seat"),
            ("billing@stripe.com", "Seat assigned to you"),
            ("hello@repair.example", "Your seat is confirmed"),
        ] {
            assert_ne!(mail("", address, subject).run().topic, Topic::Travel, "{address}: {subject}");
        }
        assert!(hinted("britishairways", "airways") && hinted("air-lines", "air") && hinted("travelfox", "travel") && hinted("bankofexample", "bank"));
        assert!(!hinted("airtable", "air") && !hinted("stripe", "trip") && !hinted("dairy", "air"));
        let gate = mail("Skyline Air", "no-reply@skyline.example", "Gate change for your flight to Seattle").text("Your flight now leaves from gate B12.");
        assert_eq!(gate.topic(), Topic::Travel, "{:?}", gate.run());
    }

    #[test]
    fn the_rules_apply_in_order() {
        let mut m = mail("Dana Whitfield", "dana@home.example", "Receipt: invoice for your order").text("Total charged: $20.00");
        m.message_override = Some(Topic::Travel);
        m.rules = vec![TopicRule { account_key: 0, pattern: "dana@home.example".into(), is_domain: false, topic: Topic::Finance }];
        m.junk = Some(Severity::Danger);
        m.message.meeting_kind = Some(MeetingMessageKind::Request);
        m.order = Some((ExtractionKind::Order, 0.8));
        m.contact = Some("Dana".into());

        assert_eq!((m.run().topic, m.run().source), (Topic::Travel, Source::MessageOverride));
        m.message_override = None;
        let rule = m.run();
        assert_eq!((rule.topic, rule.source), (Topic::Finance, Source::SenderRule));
        assert!(rule.reasons.contains(&Reason::Suspicious), "a warning still shows");
        m.rules.clear();
        assert_eq!((m.run().topic, m.run().source), (Topic::Other, Source::Suspicious));
        m.junk = None;
        assert_eq!((m.run().topic, m.run().source), (Topic::Events, Source::Meeting));
        m.message.meeting_kind = None;
        assert_eq!((m.run().topic, m.run().source), (Topic::Orders, Source::Order));
        m.order = Some((ExtractionKind::None, 0.1));
        assert_eq!((m.run().topic, m.run().source), (Topic::People, Source::Person));
        m.contact = None;
        assert_eq!((m.run().topic, m.run().source), (Topic::Receipts, Source::Signals));
    }

    #[test]
    fn people_are_people_only_when_writing_personally() {
        let mut m = mail("Dana", "dana@home.example", "Flight details").text("My flight lands at 6");
        m.replied = true;
        assert_eq!(m.topic(), Topic::People);
        let mut automated = Mail { message: Message { from: EmailAddress::new("Dana", "no-reply@home.example"), ..m.message.clone() }, ..m.clone() };
        assert_ne!(automated.topic(), Topic::People);
        automated.message.from.address = "dana@home.example".into();
        automated.message.to = vec![EmailAddress::default(); 21];
        assert_ne!(automated.topic(), Topic::People);
        let list = Mail { html: Some(newsletter("My flight lands at 6")), ..m.clone() };
        assert_ne!(list.topic(), Topic::People);
    }

    #[test]
    fn transactional_mail_with_a_footer_and_newsletters_about_money() {
        let receipt = mail("Kiln & Co", "receipts@kiln.example", "Your receipt from Kiln & Co").html(&(para(&["Subtotal $40.00", "Total $44.00"]) + &newsletter("Thanks")));
        assert_eq!(receipt.topic(), Topic::Receipts);
        let statement = mail("Harbor Bank", "news@harborbank.example", "Your monthly statement is ready").html(&newsletter("Your bank statement is ready to view."));
        assert_eq!(statement.topic(), Topic::Finance);
        let mut phish = mail("Harbor Bank", "security@harb0rbank.example", "Your bank statement is ready").html(&newsletter("Sign in to see your balance"));
        phish.junk = Some(Severity::Danger);
        assert_eq!(phish.run(), Assignment { topic: Topic::Other, confidence: 1.0, source: Source::Suspicious, reasons: vec![Reason::Suspicious] });
        phish.junk = None;
        phish.order = Some((ExtractionKind::Suppressed, 0.0));
        assert_eq!(phish.run().source, Source::Suspicious);
    }

    #[test]
    fn a_sender_posing_as_someone_never_gets_money_topics() {
        let suspicious = Assignment { topic: Topic::Other, confidence: 1.0, source: Source::Suspicious, reasons: vec![Reason::Suspicious] };
        let mut bill = mail("Harbor Bank", "alerts@harbor-bank-online.example", "Your monthly statement is ready").html(&para(&["Your bank statement is ready to view."]));
        assert_eq!(bill.topic(), Topic::Finance);
        bill.junk = Some(Severity::Caution);
        assert_eq!(bill.topic(), Topic::Finance, "a caution for something else");
        bill.impersonation = true;
        assert_eq!(bill.run(), suspicious);
        let mut receipt = mail("Kiln & Co", "receipts@kiln-shop.example", "Your receipt from Kiln & Co").html(&para(&["Subtotal $40.00", "Total $44.00"]));
        receipt.junk = Some(Severity::Caution);
        receipt.impersonation = true;
        assert_eq!(receipt.run(), suspicious);
        let mut order = mail("Brightleaf Home", "orders@brightleaf-orders.example", "Your order has shipped");
        order.order = Some((ExtractionKind::Order, 0.9));
        order.junk = Some(Severity::Caution);
        order.impersonation = true;
        assert_eq!(order.run(), suspicious);
        // Other topics stay: a newsletter is a newsletter.
        let mut news = mail("Kiln & Co", "news@kiln-shop.example", "Our autumn collection").html(&newsletter("New mugs are in"));
        let before = news.topic();
        news.junk = Some(Severity::Caution);
        news.impersonation = true;
        assert_eq!(news.topic(), before);
    }

    #[test]
    fn sender_and_domain_rules() {
        let rule = |pattern: &str, is_domain: bool, account_key: i64, topic: Topic| TopicRule { account_key, pattern: pattern.into(), is_domain, topic };
        let mut m = mail("A", "a@mail.example.com", "Hello");
        m.rules = vec![rule("example.com", true, 0, Topic::Travel)];
        assert_eq!((m.run().topic, m.run().source), (Topic::Travel, Source::DomainRule));
        m.message.from.address = "a@evilexample.com".into();
        assert_eq!(m.run().source, Source::Fallback);
        // The most specific domain, this account's before all accounts', and an address before any domain.
        m.message.from.address = "A@Mail.Example.com".into();
        m.rules.push(rule("mail.example.com", true, 0, Topic::Finance));
        assert_eq!(m.run().topic, Topic::Finance);
        m.rules.push(rule("mail.example.com", true, 1, Topic::Social));
        m.rules.push(rule("mail.example.com", true, 2, Topic::Events));
        assert_eq!(m.run().topic, Topic::Social);
        m.rules.push(rule("a@mail.example.com", false, 0, Topic::Receipts));
        assert_eq!((m.run().topic, m.run().source), (Topic::Receipts, Source::SenderRule));
        m.rules.push(rule("a@mail.example.com", false, 1, Topic::People));
        assert_eq!(m.run().topic, Topic::People);
    }

    #[test]
    fn orders_words_only_without_order_detection() {
        let m = mail("UPS", "mcinfo@ups.com", "Your package is out for delivery").text("Tracking number 1Z999AA10123456784");
        let a = m.run();
        assert_eq!(a.topic, Topic::Orders);
        assert!(a.reasons.contains(&Reason::CarrierSender));
        assert_eq!(Mail { order: Some((ExtractionKind::None, 0.3)), ..m }.run().topic, Topic::Other);
    }

    #[test]
    fn fee_lures_are_suspicious_without_order_detection() {
        let lure = mail("Parcel Desk", "tracking@parcel-desk.example", "Your order could not be delivered - pay $1.99 redelivery fee").html(
            "<p>Your order could not be delivered. Pay a small fee to reschedule: <a href=https://parcel-desk.example/pay>Pay now</a></p>",
        );
        assert_eq!(lure.run(), Assignment { topic: Topic::Other, confidence: 1.0, source: Source::Suspicious, reasons: vec![Reason::Suspicious] });
        let spoof = mail("UPS", "alerts@ups-redelivery.example", "UPS: your package is on hold").text("Confirm your address at https://ups-redelivery.example/a");
        assert_eq!(spoof.run().source, Source::Suspicious);
        // What order detection said wins when it ran.
        assert_eq!(Mail { order: Some((ExtractionKind::None, 0.3)), ..lure }.run().source, Source::Fallback);
    }

    #[test]
    fn password_mail_is_a_notification() {
        let reset = mail("Corvid Cloud", "no-reply@corvid.example", "Reset your password")
            .text("We received a request to reset your password. Use the link below to choose a new one.");
        assert_eq!(reset.topic(), Topic::Notifications, "{:?}", reset.run());
        let changed = mail("Corvid Cloud", "no-reply@corvid.example", "Your password has been changed")
            .text("Your password was changed. If this wasn't you, contact support.");
        assert_eq!(changed.topic(), Topic::Notifications, "{:?}", changed.run());
    }

    #[test]
    fn unsure_is_other() {
        let a = mail("Sky", "hi@sky.example", "Flight and hotel invitation – RSVP").run();
        assert_eq!((a.topic, a.source), (Topic::Other, Source::Fallback));
        assert!(matches!(a.reasons[..], [Reason::LowConfidence { best: Topic::Travel | Topic::Events }]), "{a:?}");
        let weak = mail("Sky", "hi@sky.example", "Your flight").run();
        assert_eq!(weak.reasons, [Reason::LowConfidence { best: Topic::Travel }]);
    }

    #[test]
    fn every_reason_describes_itself() {
        let all = [
            Reason::MessageOverride,
            Reason::SenderRule { pattern: "a@b.example".into() },
            Reason::DomainRule { pattern: "b.example".into() },
            Reason::Suspicious,
            Reason::Meeting,
            Reason::OrderEmail { merchant: String::new() },
            Reason::OrderEmail { merchant: "Brightleaf".into() },
            Reason::KnownPerson { name: String::new() },
            Reason::KnownPerson { name: "Dana".into() },
            Reason::SenderLocalPart { part: "news".into() },
            Reason::SubjectWord { word: "receipt".into() },
            Reason::BodyPhrase { phrase: "total charged".into() },
            Reason::UnsubscribeLink,
            Reason::VerificationCode,
            Reason::CarrierSender,
            Reason::LowConfidence { best: Topic::Other },
            Reason::LowConfidence { best: Topic::Travel },
        ];
        for r in all {
            assert!(!r.describe().trim().is_empty(), "{r:?}");
        }
    }

    #[test]
    fn odd_and_huge_inputs_never_panic() {
        let e = "é".repeat(200_000);
        let cjk = "領収書 発送 注文 予約確認 搭乗 ".repeat(10_000);
        let huge = "<p>Receipt total $1.00 &amp; <a href=\"https://x.example/unsubscribe\">u</a></p>".repeat(5 * 1024 * 1024 / 70);
        let cells = format!("<table><tr>{}</tr></table>", "<td>code 1234</td>".repeat(10_000));
        for m in [
            mail("", "", ""),
            mail("İİİ", "İ@İ.example", &e).text(&e),
            mail("A", "a@a.example", "注文").text(&cjk),
            mail("A", "a@a.example", "Big").html(&huge),
            mail("A", "a@a.example", "Cells").html(&cells),
            mail("A", "a@a.example", "<").html(&"<".repeat(100_000)),
            mail("A", "a@a.example", "Unclosed").html("<div><a href=\"https://x.example/<td"),
            mail("A", "a@a.example", "Big").text("This message is too large to download."),
        ] {
            let started = Instant::now();
            let a = m.run();
            assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
            assert!(!a.reasons.is_empty());
        }
    }

    #[test]
    fn a_failure_is_sorted_as_other() {
        let m = mail("Corvid Cloud", "no-reply@corvid.example", "Your verification code is 482913");
        PANIC.with(|p| p.set(true));
        let a = classify_guarded(&m.input());
        PANIC.with(|p| p.set(false));
        assert_eq!(a, fallback());
        assert_eq!(classify_guarded(&m.input()).topic, Topic::Notifications);
    }
}
