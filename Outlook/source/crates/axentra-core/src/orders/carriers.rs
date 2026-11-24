//! Tracking numbers: candidates, formats and check digits per carrier, the
//! fixed tracking-page templates, and carrier claims in the sender's name.

use super::{Carrier, MAX_TRACKING, TrackingNumber, Validation};
use crate::junk::analyze::registrable;
use crate::junk::domain_of;
use crate::scan::{Prepared, contains_word, find_word, floor_char, phone_word_before};

impl Carrier {
    /// Stable id, used in order keys: `"ups"`, `"usps"`, …, `"postal-xx"`.
    pub fn id(&self) -> String {
        match self {
            Carrier::Ups => "ups".into(),
            Carrier::Usps => "usps".into(),
            Carrier::FedEx => "fedex".into(),
            Carrier::Dhl => "dhl".into(),
            Carrier::DhlEcommerce => "dhl-ecommerce".into(),
            Carrier::AmazonLogistics => "amazon-logistics".into(),
            Carrier::RoyalMail => "royal-mail".into(),
            Carrier::CanadaPost => "canada-post".into(),
            Carrier::PostNl => "postnl".into(),
            Carrier::DeutschePost => "deutsche-post".into(),
            Carrier::LaPoste => "la-poste".into(),
            Carrier::JapanPost => "japan-post".into(),
            Carrier::ChinaPost => "china-post".into(),
            Carrier::Evri => "evri".into(),
            Carrier::Dpd => "dpd".into(),
            Carrier::OnTrac => "ontrac".into(),
            Carrier::Postal(cc) => format!("postal-{}", cc.to_ascii_lowercase()),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Carrier::Ups => "UPS".into(),
            Carrier::Usps => "USPS".into(),
            Carrier::FedEx => "FedEx".into(),
            Carrier::Dhl => "DHL".into(),
            Carrier::DhlEcommerce => "DHL eCommerce".into(),
            Carrier::AmazonLogistics => "Amazon Logistics".into(),
            Carrier::RoyalMail => "Royal Mail".into(),
            Carrier::CanadaPost => "Canada Post".into(),
            Carrier::PostNl => "PostNL".into(),
            Carrier::DeutschePost => "Deutsche Post".into(),
            Carrier::LaPoste => "La Poste".into(),
            Carrier::JapanPost => "Japan Post".into(),
            Carrier::ChinaPost => "China Post".into(),
            Carrier::Evri => "Evri".into(),
            Carrier::Dpd => "DPD".into(),
            Carrier::OnTrac => "OnTrac".into(),
            Carrier::Postal(cc) => format!("Postal service ({})", cc.to_ascii_uppercase()),
        }
    }

    /// The site [`Carrier::tracking_url`] opens ("Opens ups.com in your
    /// browser"); `None` when there is no tracking page.
    pub fn host(&self) -> Option<&'static str> {
        match self {
            Carrier::Ups => Some("ups.com"),
            Carrier::Usps => Some("usps.com"),
            Carrier::FedEx => Some("fedex.com"),
            Carrier::Dhl | Carrier::DhlEcommerce => Some("dhl.com"),
            Carrier::RoyalMail => Some("royalmail.com"),
            Carrier::CanadaPost => Some("canadapost-postescanada.ca"),
            Carrier::PostNl => Some("postnl.nl"),
            Carrier::DeutschePost => Some("deutschepost.de"),
            Carrier::LaPoste => Some("laposte.fr"),
            Carrier::JapanPost => Some("japanpost.jp"),
            Carrier::OnTrac => Some("ontrac.com"),
            Carrier::AmazonLogistics | Carrier::ChinaPost | Carrier::Evri | Carrier::Dpd | Carrier::Postal(_) => None,
        }
    }

    /// The carrier's tracking page for `number`, from a fixed template. `None`
    /// unless `number` is `[A-Z0-9]{8,40}` and passes [`validate`], and for
    /// carriers without a tracking page.
    pub fn tracking_url(&self, number: &str) -> Option<String> {
        let well_formed = (8..=40).contains(&number.len()) && number.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !well_formed || !validate(self, number) {
            return None;
        }
        let n = number;
        match self {
            Carrier::Ups => Some(format!("https://www.ups.com/track?tracknum={n}")),
            Carrier::Usps => Some(format!("https://tools.usps.com/go/TrackConfirmAction?tLabels={n}")),
            Carrier::FedEx => Some(format!("https://www.fedex.com/fedextrack/?trknbr={n}")),
            Carrier::Dhl | Carrier::DhlEcommerce => Some(format!("https://www.dhl.com/global-en/home/tracking/tracking-parcel.html?tracking-id={n}")),
            Carrier::RoyalMail => Some(format!("https://www.royalmail.com/track-your-item#/tracking-results/{n}")),
            Carrier::CanadaPost => Some(format!("https://www.canadapost-postescanada.ca/track-reperage/en#/search?searchFor={n}")),
            Carrier::PostNl => Some(format!("https://jouw.postnl.nl/track-and-trace/{n}")),
            Carrier::DeutschePost => Some(format!("https://www.deutschepost.de/de/s/sendungsverfolgung.html?piececode={n}")),
            Carrier::LaPoste => Some(format!("https://www.laposte.fr/outils/suivre-vos-envois?code={n}")),
            Carrier::JapanPost => Some(format!("https://trackings.post.japanpost.jp/services/srv/search/?requestNo1={n}")),
            Carrier::OnTrac => Some(format!("https://www.ontrac.com/tracking/?number={n}")),
            Carrier::AmazonLogistics | Carrier::ChinaPost | Carrier::Evri | Carrier::Dpd | Carrier::Postal(_) => None,
        }
    }
}

/// True when `number` has the carrier's format and (where it has one) a
/// matching check digit.
pub fn validate(carrier: &Carrier, number: &str) -> bool {
    let n = number;
    match carrier {
        Carrier::Ups => ups_ok(n),
        Carrier::Usps => impb_core(n) == Some(n) || s10_country(n) == Some("US"),
        Carrier::FedEx => fedex12_ok(n) || fedex15_ok(n) || sscc_ok(n),
        Carrier::Dhl => dhl10_ok(n) || jjd(n),
        Carrier::DhlEcommerce => gm(n),
        Carrier::AmazonLogistics => tba(n),
        Carrier::RoyalMail => s10_country(n) == Some("GB"),
        Carrier::CanadaPost => digits_of_len(n, 16) || s10_country(n) == Some("CA"),
        Carrier::PostNl => postnl(n) || s10_country(n) == Some("NL"),
        Carrier::DeutschePost => s10_country(n) == Some("DE"),
        Carrier::LaPoste => s10_country(n) == Some("FR"),
        Carrier::JapanPost => s10_country(n) == Some("JP"),
        Carrier::ChinaPost => s10_country(n) == Some("CN"),
        Carrier::Evri => evri(n),
        Carrier::Dpd => digits_of_len(n, 14),
        Carrier::OnTrac => ontrac(n),
        Carrier::Postal(cc) => s10_country(n) == Some(cc.as_str()),
    }
}

/// Candidates looked at per message at most.
const MAX_CANDIDATES: usize = 200;

/// Words that put a tracking number next to them.
const TRACKING_LABELS: &[&str] = &[
    "tracking number",
    "tracking #",
    "tracking no",
    "tracking id",
    "track your",
    "sendungsnummer",
    "numéro de suivi",
    "numero de suivi",
    "número de seguimiento",
    "numero de seguimiento",
    "追跡番号",
    "お問い合わせ番号",
    "运单号",
    "快递单号",
];

/// The carriers with a name, in the order a claim is looked for.
const NAMED: [Carrier; 16] = [
    Carrier::Ups,
    Carrier::Usps,
    Carrier::FedEx,
    Carrier::Dhl,
    Carrier::DhlEcommerce,
    Carrier::AmazonLogistics,
    Carrier::RoyalMail,
    Carrier::CanadaPost,
    Carrier::PostNl,
    Carrier::DeutschePost,
    Carrier::LaPoste,
    Carrier::JapanPost,
    Carrier::ChinaPost,
    Carrier::Evri,
    Carrier::Dpd,
    Carrier::OnTrac,
];

/// Registrable domains the carrier sends mail from.
fn sender_domains(c: &Carrier) -> &'static [&'static str] {
    match c {
        Carrier::Ups => &["ups.com"],
        Carrier::Usps => &["usps.com", "usps.gov"],
        Carrier::FedEx => &["fedex.com"],
        Carrier::Dhl | Carrier::DhlEcommerce => &["dhl.com", "dhl.de", "dhl.co.uk"],
        Carrier::AmazonLogistics => &["amazon.com", "amazon.co.uk", "amazon.de", "amazon.fr", "amazon.es", "amazon.it", "amazon.ca", "amazon.co.jp"],
        Carrier::RoyalMail => &["royalmail.com", "royalmail.co.uk"],
        Carrier::CanadaPost => &["canadapost.ca", "canadapost-postescanada.ca"],
        Carrier::PostNl => &["postnl.nl"],
        Carrier::DeutschePost => &["deutschepost.de"],
        Carrier::LaPoste => &["laposte.fr"],
        Carrier::JapanPost => &["japanpost.jp", "post.japanpost.jp"],
        Carrier::Evri => &["evri.com"],
        Carrier::Dpd => &["dpd.com", "dpd.co.uk", "dpd.de"],
        Carrier::OnTrac => &["ontrac.com"],
        Carrier::ChinaPost | Carrier::Postal(_) => &[],
    }
}

/// How the carrier is named in mail (ASCII lowercase, or as written).
fn names(c: &Carrier) -> &'static [&'static str] {
    match c {
        Carrier::Ups => &["ups"],
        Carrier::Usps => &["usps", "postal service"],
        Carrier::FedEx => &["fedex"],
        Carrier::Dhl | Carrier::DhlEcommerce => &["dhl"],
        Carrier::AmazonLogistics => &["amazon"],
        Carrier::RoyalMail => &["royal mail"],
        Carrier::CanadaPost => &["canada post", "postes canada"],
        Carrier::PostNl => &["postnl"],
        Carrier::DeutschePost => &["deutsche post"],
        Carrier::LaPoste => &["la poste", "colissimo"],
        Carrier::JapanPost => &["japan post", "日本郵便"],
        Carrier::ChinaPost => &["china post", "中国邮政"],
        Carrier::Evri => &["evri"],
        Carrier::Dpd => &["dpd"],
        Carrier::OnTrac => &["ontrac"],
        Carrier::Postal(_) => &[],
    }
}

/// Every registrable domain a carrier sends mail from.
pub(crate) fn carrier_sender_domains() -> Vec<&'static str> {
    NAMED.iter().flat_map(sender_domains).copied().collect()
}

/// The carrier that sends mail from `registrable_domain`, if any.
pub(crate) fn carrier_sender(registrable_domain: &str) -> Option<Carrier> {
    NAMED.into_iter().find(|c| sender_domains(c).contains(&registrable_domain))
}

/// True when `registrable_domain` is one the carrier sends mail from.
pub(crate) fn sends_for(c: &Carrier, registrable_domain: &str) -> bool {
    sender_domains(c).contains(&registrable_domain)
}

/// The tracking numbers in a message (deduped, at most `MAX_TRACKING`), and
/// whether a cap (candidates or results) was hit.
pub(crate) fn tracking(p: &Prepared, sender: &str, order_number: Option<&str>) -> (Vec<TrackingNumber>, bool) {
    let links = p
        .links
        .iter()
        .filter_map(|(href, _)| host_of(href).map(|h| Link { reg: registrable(&h), upper: href.to_ascii_uppercase() }))
        .collect();
    let mut s = Scan { sender: registrable(domain_of(&sender.trim().to_ascii_lowercase())), links, order_number, found: Vec::new(), candidates: 0, capped: false };
    s.text(&p.subject, &p.subject.to_ascii_lowercase());
    s.text(&p.body, &p.body_lower);
    s.links(&p.links);
    let capped = s.capped || s.found.len() > MAX_TRACKING;
    s.found.truncate(MAX_TRACKING);
    (s.found, capped)
}

struct Link {
    /// Registrable domain of the host.
    reg: String,
    upper: String,
}

struct Scan<'a> {
    /// Registrable domain of the sender.
    sender: String,
    links: Vec<Link>,
    order_number: Option<&'a str>,
    found: Vec<TrackingNumber>,
    candidates: usize,
    capped: bool,
}

impl Scan<'_> {
    /// Candidates in `text`: runs of 8–40 letters and digits, and digit
    /// groups after a tracking label ("9400 1118 9922 …").
    fn text(&mut self, text: &str, lower: &str) {
        let b = text.as_bytes();
        let mut quoted = quoted_line(text, 0);
        let mut i = 0;
        // The end of the last digit-group run tried as a whole: its later
        // groups never start a joined number of their own, so the tail of a
        // number ("3333 4444 59 10") is not read as a shorter one.
        let mut run_end = 0;
        while i < b.len() && !self.capped {
            if b[i] == b'\n' {
                i += 1;
                quoted = quoted_line(text, i);
                continue;
            }
            if !b[i].is_ascii_alphanumeric() {
                i += 1;
                continue;
            }
            let s = i;
            while i < b.len() && b[i].is_ascii_alphanumeric() {
                i += 1;
            }
            if s >= run_end && b[s..i].iter().all(u8::is_ascii_digit) && label_before(lower, s, 40) {
                let (digits, groups) = digit_groups(b, s);
                run_end = groups.last().map_or(i, |g| g.1);
                // The whole run, else the longest run of leading groups that
                // is a number: "9400 … 59 2 items" is the number then a count.
                let kept = (2..=groups.len()).rev().map(|n| groups[n - 1]).find(|&(len, end)| {
                    (8..=40).contains(&len) && self.consider(text, lower, s, end, &digits[..len], quoted)
                });
                if let Some((_, end)) = kept {
                    i = end;
                    continue;
                }
            }
            // Every format has a digit: plain words ("information",
            // "instructions") never count against the cap.
            if (8..=40).contains(&(i - s)) && b[s..i].iter().any(u8::is_ascii_digit) {
                self.consider(text, lower, s, i, &text[s..i].to_ascii_uppercase(), quoted);
            }
        }
    }

    /// Numbers in links to a carrier's tracking page. Only the number is
    /// kept; the link itself is never stored, shown or followed.
    fn links(&mut self, links: &[(String, String)]) {
        for (href, _) in links {
            let Some(host) = host_of(href) else { continue };
            let (carrier, params): (Carrier, &[&str]) = match registrable(&host).as_str() {
                "ups.com" => (Carrier::Ups, &["tracknum", "inquirynumber1"]),
                "usps.com" => (Carrier::Usps, &["tlabels", "qtc_tlabels1"]),
                "fedex.com" => (Carrier::FedEx, &["trknbr", "tracknumbers"]),
                "dhl.com" => (Carrier::Dhl, &["tracking-id", "awb"]),
                "royalmail.com" => (Carrier::RoyalMail, &[]),
                _ => continue,
            };
            for value in link_values(href, params) {
                if self.capped {
                    return;
                }
                self.candidates += 1;
                if self.candidates > MAX_CANDIDATES {
                    self.capped = true;
                    return;
                }
                if self.order_number == Some(value.as_str()) {
                    continue;
                }
                let family = |c: &Carrier| same_family(c, &carrier);
                if let Some((carrier, number, _)) = resolve(&value, false, false, &family) {
                    self.push(TrackingNumber { carrier, number, validated: Validation::Link });
                }
            }
        }
    }

    /// Looks at one candidate at `s..e` of `text`; true when it was kept.
    fn consider(&mut self, text: &str, lower: &str, s: usize, e: usize, number: &str, quoted: bool) -> bool {
        self.candidates += 1;
        if self.candidates > MAX_CANDIDATES {
            self.capped = true;
            return false;
        }
        let digits = number.bytes().all(|b| b.is_ascii_digit());
        if quoted || in_address(text.as_bytes(), s, e) || self.order_number == Some(number) || (digits && phone_word_before(lower, s)) || self.pixel(number) {
            return false;
        }
        let has = |c: &Carrier| {
            sends_for(c, &self.sender) || self.links.iter().any(|l| sends_for(c, &l.reg)) || names(c).iter().any(|n| word_near(lower, s, e, n))
        };
        let label = label_before(lower, s, 40);
        let Some((carrier, number, validated)) = resolve(number, label, label || label_before(lower, s, 80), &has) else { return false };
        self.push(TrackingNumber { carrier, number, validated });
        true
    }

    /// The same token in links to three or more other sites: a tracking
    /// pixel or click id, not a parcel.
    fn pixel(&self, number: &str) -> bool {
        let mut hosts: Vec<&str> = Vec::new();
        for l in &self.links {
            if carrier_sender(&l.reg).is_none() && l.upper.contains(number) && !hosts.contains(&l.reg.as_str()) {
                hosts.push(&l.reg);
            }
        }
        hosts.len() >= 3
    }

    /// Adds `t`, or upgrades how an already found number was confirmed.
    fn push(&mut self, t: TrackingNumber) {
        let rank = |v: Validation| match v {
            Validation::CheckDigit => 2,
            Validation::Link => 1,
            Validation::Labelled => 0,
        };
        match self.found.iter_mut().find(|f| f.number == t.number) {
            Some(f) if rank(t.validated) > rank(f.validated) => f.validated = t.validated,
            Some(_) => {}
            None => self.found.push(t),
        }
    }
}

/// The first carrier (in table order) that accepts `n`, given whether a
/// tracking label is near (`label`: 40 bytes, `label_wide`: 80) and which
/// carriers the message mentions (`has`).
fn resolve(n: &str, label: bool, label_wide: bool, has: &dyn Fn(&Carrier) -> bool) -> Option<(Carrier, String, Validation)> {
    use Validation::{CheckDigit, Labelled};
    let found = |c: Carrier, v: Validation| Some((c, n.to_string(), v));
    if ups_ok(n) {
        return found(Carrier::Ups, CheckDigit);
    }
    if let Some(core) = impb_core(n) {
        return Some((Carrier::Usps, core.to_string(), CheckDigit));
    }
    if let Some(c) = s10_carrier(n) {
        return found(c, CheckDigit);
    }
    if (fedex12_ok(n) || fedex15_ok(n) || sscc_ok(n)) && (label || has(&Carrier::FedEx)) {
        return found(Carrier::FedEx, CheckDigit);
    }
    if dhl10_ok(n) && (label_wide || has(&Carrier::Dhl)) {
        return found(Carrier::Dhl, CheckDigit);
    }
    if gm(n) && (label || has(&Carrier::DhlEcommerce)) {
        return found(Carrier::DhlEcommerce, Labelled);
    }
    if jjd(n) && (label || has(&Carrier::Dhl)) {
        return found(Carrier::Dhl, Labelled);
    }
    if tba(n) && (label || has(&Carrier::AmazonLogistics)) {
        return found(Carrier::AmazonLogistics, Labelled);
    }
    if digits_of_len(n, 16) && label && has(&Carrier::CanadaPost) {
        return found(Carrier::CanadaPost, Labelled);
    }
    if postnl(n) && (label || has(&Carrier::PostNl)) {
        return found(Carrier::PostNl, Labelled);
    }
    if evri(n) && label && has(&Carrier::Evri) {
        return found(Carrier::Evri, Labelled);
    }
    if digits_of_len(n, 14) && label && has(&Carrier::Dpd) {
        return found(Carrier::Dpd, Labelled);
    }
    if ontrac(n) && label && has(&Carrier::OnTrac) {
        return found(Carrier::OnTrac, Labelled);
    }
    None
}

fn same_family(a: &Carrier, b: &Carrier) -> bool {
    let family = |c: &Carrier| match c {
        Carrier::DhlEcommerce => Carrier::Dhl,
        c => c.clone(),
    };
    family(a) == family(b)
}

/// True when the line starting at `start` is quoted (`>`).
fn quoted_line(text: &str, start: usize) -> bool {
    text[start..].trim_start_matches([' ', '\t']).starts_with('>')
}

/// A tracking label ends within `width` bytes before `at`.
fn label_before(lower: &str, at: usize, width: usize) -> bool {
    let window = &lower[floor_char(lower, at.saturating_sub(width))..at];
    TRACKING_LABELS.iter().any(|l| window.contains(l))
}

/// `name` as a word within 80 bytes of `s..e`.
fn word_near(lower: &str, s: usize, e: usize, name: &str) -> bool {
    let window = &lower[floor_char(lower, s.saturating_sub(80))..floor_char(lower, e + 80)];
    contains_word(window, name)
}

/// The token around `s..e` (up to 64 bytes each side) is an email address.
fn in_address(b: &[u8], s: usize, e: usize) -> bool {
    let stop = |c: u8| c.is_ascii_whitespace() || matches!(c, b'<' | b'>' | b'(' | b')' | b'"' | b'\'' | b',' | b';');
    let before = b[s.saturating_sub(64)..s].iter().rev().take_while(|c| !stop(**c)).any(|c| *c == b'@');
    let after = b[e..(e + 64).min(b.len())].iter().take_while(|c| !stop(**c)).any(|c| *c == b'@');
    before || after
}

/// Digit groups from `s` joined across single spaces or hyphens (at most
/// 40 digits): the digits, and for each group the digit count so far and
/// where the group ends.
fn digit_groups(b: &[u8], s: usize) -> (String, Vec<(usize, usize)>) {
    let mut digits = String::new();
    let mut groups = Vec::new();
    let mut i = s;
    loop {
        let g = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == g || (i < b.len() && b[i].is_ascii_alphabetic()) || digits.len() + (i - g) > 40 {
            break;
        }
        digits.extend(b[g..i].iter().map(|c| *c as char));
        groups.push((digits.len(), i));
        if i + 1 < b.len() && matches!(b[i], b' ' | b'-') && b[i + 1].is_ascii_digit() {
            i += 1;
        } else {
            break;
        }
    }
    (digits, groups)
}

/// The host of an http(s) link, lowercase, without user info or port.
fn host_of(href: &str) -> Option<String> {
    let lower = href.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    (!host.is_empty()).then(|| host.to_string())
}

/// Tracking numbers in a carrier link: the values of `params` in its query
/// (comma-separated values each count), or the path after
/// `tracking-results/` when `params` is empty.
fn link_values(href: &str, params: &[&str]) -> Vec<String> {
    let well_formed = |v: &str| (8..=40).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric());
    let mut out = Vec::new();
    if params.is_empty() {
        let lower = href.to_ascii_lowercase();
        if let Some(i) = lower.find("tracking-results/") {
            let v: String = href[i + "tracking-results/".len()..].chars().take_while(char::is_ascii_alphanumeric).collect();
            if well_formed(&v) {
                out.push(v.to_ascii_uppercase());
            }
        }
        return out;
    }
    let Some((_, query)) = href.split_once('?') else { return out };
    let query = query.split('#').next().unwrap_or("");
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if !params.iter().any(|p| k.eq_ignore_ascii_case(p)) {
            continue;
        }
        for v in v.split(',').flat_map(|v| v.split("%2C")).flat_map(|v| v.split("%2c")) {
            let v = v.trim();
            if well_formed(v) {
                out.push(v.to_ascii_uppercase());
            }
        }
    }
    out
}

/// A carrier named in the sender's name or the start of the subject (never
/// the body), and whether it was the sender's name. Only a name: whether the
/// message is a parcel notice at all is the caller's to judge.
pub(crate) fn claimed_carrier(from_name: &str, subject: &str) -> Option<(Carrier, bool)> {
    let start = &subject[..floor_char(subject, 40)];
    [(from_name, true), (start, false)].into_iter().find_map(|(field, in_name)| {
        let lower = field.to_ascii_lowercase();
        let named = |n: &str| named_in(field, &lower, n);
        NAMED
            .into_iter()
            .find(|c| match c {
                // "Amazon" alone is a shop; only its delivery service is a carrier.
                Carrier::AmazonLogistics => {
                    named("amazon") && ["delivery", "deliveries", "shipping", "shipment", "logistics", "package", "parcel"].iter().any(|w| contains_word(&lower, w))
                }
                // Named by its plain "DHL".
                Carrier::DhlEcommerce | Carrier::ChinaPost => false,
                Carrier::Usps => named("usps"),
                c => names(c).iter().any(|n| named(n)),
            })
            .map(|c| (c, in_name))
    })
}

/// `name` in `field` as a word of its own: not joined by a hyphen
/// ("follow-ups", "sign-ups"), and the short acronyms only in capitals
/// ("UPS", not the German "Ups").
fn named_in(field: &str, lower: &str, name: &str) -> bool {
    let acronym = matches!(name, "ups" | "dhl" | "dpd");
    find_word(lower, name).any(|i| {
        let e = i + name.len();
        let hyphen = lower[..i].ends_with('-') || lower[e..].starts_with('-');
        !hyphen && (!acronym || field[i..e].bytes().all(|b| b.is_ascii_uppercase()))
    })
}

// Formats and check digits.

fn digits_of_len(n: &str, len: usize) -> bool {
    n.len() == len && n.bytes().all(|b| b.is_ascii_digit())
}

fn digit_values(s: &str) -> Option<Vec<u32>> {
    s.chars().map(|c| c.to_digit(10)).collect()
}

/// Mod 10 with weights 3, 1, 3, … from the right (USPS IMpb, FedEx Ground).
fn mod10_31(body: &str) -> Option<u32> {
    let d = digit_values(body)?;
    let sum: u32 = d.iter().rev().enumerate().map(|(i, v)| v * if i % 2 == 0 { 3 } else { 1 }).sum();
    Some((10 - sum % 10) % 10)
}

fn check_char(n: &str, check: Option<u32>) -> bool {
    check.is_some_and(|c| n.as_bytes().last().is_some_and(|b| u32::from(*b) == u32::from(b'0') + c))
}

fn ups_check(body15: &str) -> Option<u32> {
    if body15.len() != 15 {
        return None;
    }
    let mut sum = 0;
    for (i, c) in body15.chars().enumerate() {
        let v = match c {
            '0'..='9' => c as u32 - '0' as u32,
            'A'..='Z' => (c as u32 - 63) % 10,
            _ => return None,
        };
        // Positions are 1-based: even ones count twice.
        sum += if i % 2 == 1 { 2 * v } else { v };
    }
    Some((10 - sum % 10) % 10)
}

fn ups_ok(n: &str) -> bool {
    n.len() == 18 && n.starts_with("1Z") && n.is_ascii() && check_char(n, ups_check(&n[2..17]))
}

/// The IMpb number in `n` (without a leading `420`+ZIP routing code), when
/// its check digit matches.
fn impb_core(n: &str) -> Option<&str> {
    if !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ok = |c: &str| {
        matches!(c.len(), 20 | 22 | 26 | 30 | 34) && c.starts_with('9') && matches!(c.as_bytes()[1], b'1'..=b'5') && check_char(c, mod10_31(&c[..c.len() - 1]))
    };
    if let Some(rest) = n.strip_prefix("420") {
        for zip in [5, 9] {
            if rest.len() > zip && ok(&rest[zip..]) {
                return Some(&rest[zip..]);
            }
        }
    }
    ok(n).then_some(n)
}

fn s10_check(serial8: &str) -> Option<u32> {
    let d = digit_values(serial8).filter(|d| d.len() == 8)?;
    let sum: u32 = d.iter().zip([8, 6, 4, 2, 3, 5, 9, 7]).map(|(v, w)| v * w).sum();
    Some(match 11 - sum % 11 {
        10 => 0,
        11 => 5,
        c => c,
    })
}

/// The country code of a valid UPU S10 number (`^[A-Z]{2}\d{9}[A-Z]{2}$`).
fn s10_country(n: &str) -> Option<&str> {
    let b = n.as_bytes();
    let shape = b.len() == 13 && b[..2].iter().chain(&b[11..]).all(u8::is_ascii_uppercase) && b[2..11].iter().all(u8::is_ascii_digit);
    (shape && check_char(&n[..11], s10_check(&n[2..10]))).then(|| &n[11..])
}

fn s10_carrier(n: &str) -> Option<Carrier> {
    Some(match s10_country(n)? {
        "US" => Carrier::Usps,
        "GB" => Carrier::RoyalMail,
        "DE" => Carrier::DeutschePost,
        "FR" => Carrier::LaPoste,
        "NL" => Carrier::PostNl,
        "CA" => Carrier::CanadaPost,
        "JP" => Carrier::JapanPost,
        "CN" => Carrier::ChinaPost,
        cc => Carrier::Postal(cc.to_string()),
    })
}

fn fedex12_check(d11: &str) -> Option<u32> {
    let d = digit_values(d11).filter(|d| d.len() == 11)?;
    let sum: u32 = d.iter().rev().zip([1, 3, 7].into_iter().cycle()).map(|(v, w)| v * w).sum();
    Some(sum % 11 % 10)
}

fn fedex12_ok(n: &str) -> bool {
    digits_of_len(n, 12) && check_char(n, fedex12_check(&n[..11]))
}

fn fedex15_ok(n: &str) -> bool {
    digits_of_len(n, 15) && check_char(n, mod10_31(&n[..14]))
}

/// FedEx SSCC: 20 or 22 digits starting `96`, the last 15 a Ground number.
fn sscc_ok(n: &str) -> bool {
    (digits_of_len(n, 20) || digits_of_len(n, 22)) && n.starts_with("96") && fedex15_ok(&n[n.len() - 15..])
}

fn dhl10_check(d9: &str) -> Option<u32> {
    let v: u64 = d9.parse().ok().filter(|_| digits_of_len(d9, 9))?;
    Some((v % 7) as u32)
}

fn dhl10_ok(n: &str) -> bool {
    digits_of_len(n, 10) && check_char(n, dhl10_check(&n[..9]))
}

/// `prefix` then `min..=max` digits.
fn prefixed_digits(n: &str, prefix: &str, min: usize, max: usize) -> bool {
    n.strip_prefix(prefix).is_some_and(|d| (min..=max).contains(&d.len()) && d.bytes().all(|b| b.is_ascii_digit()))
}

fn gm(n: &str) -> bool {
    prefixed_digits(n, "GM", 16, 18)
}

fn jjd(n: &str) -> bool {
    prefixed_digits(n, "JJD", 18, 24)
}

fn tba(n: &str) -> bool {
    prefixed_digits(n, "TBA", 12, 12)
}

fn ontrac(n: &str) -> bool {
    prefixed_digits(n, "C", 14, 14) || prefixed_digits(n, "D", 14, 14)
}

fn postnl(n: &str) -> bool {
    n.strip_prefix("3S").is_some_and(|r| (11..=13).contains(&r.len()) && r.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()))
}

fn evri(n: &str) -> bool {
    n.len() == 16 && n.starts_with('H') && n.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

// Test helpers (also used by the demo data's tests): a body with its check
// digit.

#[cfg(test)]
fn with_check(body: &str, check: Option<u32>) -> String {
    check.map(|c| format!("{body}{c}")).unwrap_or_default()
}

/// `"1Z"` + `body15` + the UPS check digit.
#[cfg(test)]
pub(crate) fn ups_with_check(body15: &str) -> String {
    with_check(&format!("1Z{body15}"), ups_check(body15))
}

/// A USPS IMpb number: `body` + its check digit.
#[cfg(test)]
pub(crate) fn impb_with_check(body: &str) -> String {
    with_check(body, mod10_31(body))
}

/// A UPU S10 number: `prefix2` + `serial8` + check digit + `cc`.
#[cfg(test)]
pub(crate) fn s10_with_check(prefix2: &str, serial8: &str, cc: &str) -> String {
    s10_check(serial8).map(|c| format!("{prefix2}{serial8}{c}{cc}")).unwrap_or_default()
}

/// A 12-digit FedEx Express number: `d11` + its check digit.
#[cfg(test)]
pub(crate) fn fedex12_with_check(d11: &str) -> String {
    with_check(d11, fedex12_check(d11))
}

/// A 10-digit DHL Express number: `d9` + its check digit.
#[cfg(test)]
pub(crate) fn dhl10_with_check(d9: &str) -> String {
    with_check(d9, dhl10_check(d9))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EmailAddress, Message};
    use crate::scan::prepare;

    /// The tracking numbers found in a plain-text body.
    fn found(sender: &str, text: &str) -> Vec<TrackingNumber> {
        found_html(sender, None, text)
    }

    fn found_html(sender: &str, html: Option<&str>, text: &str) -> Vec<TrackingNumber> {
        let m = Message { from: EmailAddress::new("", sender), subject: "Hello".into(), ..Default::default() };
        tracking(&prepare(&m, html, text), sender, None).0
    }

    fn numbers(t: &[TrackingNumber]) -> Vec<(Carrier, &str)> {
        t.iter().map(|t| (t.carrier.clone(), t.number.as_str())).collect()
    }

    /// Known-good numbers: the UPS sample and the demo data's (spec §6.2).
    const GOOD: &[(Carrier, &str)] = &[
        (Carrier::Ups, "1Z999AA10123456784"),
        (Carrier::Ups, "1Z4X7Q203660123459"),
        (Carrier::Ups, "1Z8F29E10355042072"),
        (Carrier::FedEx, "612900344789"),
        (Carrier::FedEx, "550147029134"),
        (Carrier::Usps, "9200190301123456789017"),
        (Carrier::JapanPost, "EJ517382041JP"),
        (Carrier::Usps, "LX204417384US"),
        (Carrier::CanadaPost, "CP319475029CA"),
        (Carrier::Usps, "RR123456785US"),
        (Carrier::Dhl, "3720419861"),
    ];

    /// Errors a carrier's check digit can't see by construction: UPS
    /// doubles every other character (a change of 5 vanishes mod 10) and
    /// never weighs its check digit against the last character; DHL's mod 7
    /// misses a change of 7; S10 folds two remainders into each of 0 and 5,
    /// and its letters carry no check.
    fn blind_spot(carrier: &Carrier, good: &str, bad: &str) -> bool {
        let diff: Vec<usize> = (0..good.len()).filter(|&i| good.as_bytes()[i] != bad.as_bytes()[i]).collect();
        let delta = |i: usize| (i32::from(good.as_bytes()[i]) - i32::from(bad.as_bytes()[i])).abs();
        match carrier {
            Carrier::Ups => match diff[..] {
                [i] => i % 2 == 1 && delta(i) == 5,
                [16, 17] => true,
                _ => false,
            },
            Carrier::Dhl => diff.len() == 1 && delta(diff[0]) == 7,
            _ if good.len() == 13 => diff.iter().any(|&i| !(2..11).contains(&i)) || matches!(&good[10..11], "0" | "5") || matches!(&bad[10..11], "0" | "5"),
            _ => false,
        }
    }

    #[test]
    fn validators_accept_known_numbers_and_catch_typos() {
        assert_eq!(ups_with_check("999AA1012345678"), "1Z999AA10123456784");
        assert_eq!(ups_with_check("4X7Q20366012345"), "1Z4X7Q203660123459");
        assert_eq!(ups_with_check("8F29E1035504207"), "1Z8F29E10355042072");
        assert_eq!(fedex12_with_check("61290034478"), "612900344789");
        assert_eq!(fedex12_with_check("55014702913"), "550147029134");
        assert_eq!(impb_with_check("920019030112345678901"), "9200190301123456789017");
        assert_eq!(s10_with_check("EJ", "51738204", "JP"), "EJ517382041JP");
        assert_eq!(s10_with_check("LX", "20441738", "US"), "LX204417384US");
        assert_eq!(s10_with_check("CP", "31947502", "CA"), "CP319475029CA");
        assert_eq!(s10_with_check("RR", "12345678", "US"), "RR123456785US");
        assert_eq!(dhl10_with_check("372041986"), "3720419861");
        for (carrier, n) in GOOD {
            assert!(validate(carrier, n), "{n}");
            let mut bad = Vec::new();
            for (i, c) in n.char_indices().filter(|(_, c)| c.is_ascii_digit()) {
                for d in ('0'..='9').filter(|d| *d != c) {
                    bad.push(format!("{}{d}{}", &n[..i], &n[i + 1..]));
                }
            }
            for i in 0..n.len() - 1 {
                let mut b = n.as_bytes().to_vec();
                b.swap(i, i + 1);
                let swapped = String::from_utf8(b).unwrap();
                if swapped != *n {
                    bad.push(swapped);
                }
            }
            for b in bad {
                assert!(!validate(carrier, &b) || blind_spot(carrier, n, &b), "{n} → {b} still validates");
            }
        }
        assert!(!validate(&Carrier::Ups, "1Z999AA10123456785"));
    }

    #[test]
    fn s10_numbers_are_thirteen_characters() {
        assert!(validate(&Carrier::Usps, "RR123456785US"));
        assert_eq!(numbers(&found("a@shop.example", "Tracking number: RR123456785US")), [(Carrier::Usps, "RR123456785US")]);
        // Eight serial digits and no check digit: the old, wrong pattern.
        assert!(!validate(&Carrier::Usps, "RR12345678US"));
        assert!(found("a@shop.example", "Tracking number: RR12345678US").is_empty());
        assert_eq!(numbers(&found("a@shop.example", "追跡番号:EJ517382041JP")), [(Carrier::JapanPost, "EJ517382041JP")]);
        assert_eq!(numbers(&found("a@shop.example", "Tracking number: AB123456785ZZ")), [(Carrier::Postal("ZZ".into()), "AB123456785ZZ")]);
    }

    #[test]
    fn short_numbers_need_context() {
        assert!(found("a@shop.example", "Reference 3720419861 for your records").is_empty());
        assert_eq!(numbers(&found("a@shop.example", "DHL tracking 3720419861")), [(Carrier::Dhl, "3720419861")]);
        assert!(found("a@shop.example", "Invoice 612900344789 attached").is_empty());
        assert_eq!(numbers(&found("a@shop.example", "FedEx tracking number: 612900344789")), [(Carrier::FedEx, "612900344789")]);
        // The carrier's own mail is context too.
        assert_eq!(numbers(&found("track@fedex.com", "Shipment 612900344789")), [(Carrier::FedEx, "612900344789")]);
        // Long formats are recognised anywhere, short labelled ones only with the carrier named.
        assert_eq!(found("a@shop.example", "1Z999AA10123456784")[0].validated, Validation::CheckDigit);
        assert!(found("a@shop.example", "Tracking number: 1234567890123456").is_empty());
        let cp = found("a@shop.example", "Canada Post tracking number: 1234567890123456");
        assert_eq!(numbers(&cp), [(Carrier::CanadaPost, "1234567890123456")]);
        assert_eq!(cp[0].validated, Validation::Labelled);
        assert_eq!(numbers(&found("a@shop.example", "Amazon tracking: TBA123456789012")), [(Carrier::AmazonLogistics, "TBA123456789012")]);
    }

    #[test]
    fn digit_groups_join_after_a_label() {
        let t = found("a@shop.example", "USPS tracking number: 9200 1903 0112 3456 7890 17\nThanks!");
        assert_eq!(numbers(&t), [(Carrier::Usps, "9200190301123456789017")]);
        // A ZIP routing prefix is dropped.
        let zip = format!("420{}{}", "12345", "9200190301123456789017");
        assert_eq!(numbers(&found("a@shop.example", &format!("Tracking: {zip}"))), [(Carrier::Usps, "9200190301123456789017")]);
        // Without a label, groups stay apart (and are too short alone).
        assert!(found("a@shop.example", "Call 9200 1903 0112 3456 7890 17").is_empty());
    }

    #[test]
    fn a_count_or_date_after_spaced_digits_is_not_part_of_the_number() {
        let impb = impb_with_check("940011189922333344445");
        let spaced = format!("{} {} {} {} {} {}", &impb[..4], &impb[4..8], &impb[8..12], &impb[12..16], &impb[16..20], &impb[20..]);
        for tail in [" 2 items", " 10/22", " 7 8"] {
            let t = found("a@shop.example", &format!("USPS tracking number: {spaced}{tail}"));
            assert_eq!(numbers(&t), [(Carrier::Usps, impb.as_str())], "{tail}");
        }
        // The tail of a run that is no number at all is not tried on its own:
        // "3333 4444 59 10" passes the FedEx check but is the end of a longer run.
        let fedex_tail = "333344445910";
        assert!(fedex12_ok(fedex_tail));
        assert!(found("a@shop.example", "FedEx tracking number: 1234 5678 3333 4444 59 10/22").iter().all(|t| t.number != fedex_tail));
    }

    #[test]
    fn look_alikes_are_rejected() {
        assert!(found("a@shop.example", "Tracking number questions? Call 1-800-555-0199").is_empty());
        assert!(found("a@shop.example", "Tracking number: call 18005550199").is_empty());
        let m = Message { from: EmailAddress::new("", "a@shop.example"), subject: "Hi".into(), ..Default::default() };
        let p = prepare(&m, None, "Order 1Z999AA10123456784");
        assert!(tracking(&p, "a@shop.example", Some("1Z999AA10123456784")).0.is_empty());
        assert!(found("a@shop.example", "Write to x1Z999AA10123456784@a.example").is_empty());
        assert!(found("a@shop.example", "Write to 1Z999AA10123456784@a.example").is_empty());
        assert!(found("a@shop.example", "On Monday you wrote:\n> Tracking number: 1Z999AA10123456784\nThanks").is_empty());
        let pixels: String =
            ["a.example", "b.example", "c.example"].iter().map(|h| format!("<a href=\"https://{h}/p?id=1Z999AA10123456784\">x</a>")).collect::<String>() + "<p>Ref 1Z999AA10123456784</p>";
        assert!(found_html("a@shop.example", Some(&pixels), "").is_empty());
        // Two other sites are fine.
        let two: String = ["a.example", "b.example"].iter().map(|h| format!("<a href=\"https://{h}/p?id=1Z999AA10123456784\">x</a>")).collect::<String>() + "<p>Ref 1Z999AA10123456784</p>";
        assert_eq!(found_html("a@shop.example", Some(&two), "").len(), 1);
    }

    #[test]
    fn numbers_are_read_from_carrier_links_only() {
        let html = r#"<p>Your parcel is on its way.</p><a href="https://www.ups.com/track?loc=en_US&tracknum=1Z999AA10123456784&requester=ST">Track</a>"#;
        let t = found_html("a@shop.example", Some(html), "");
        assert_eq!(numbers(&t), [(Carrier::Ups, "1Z999AA10123456784")]);
        assert_eq!(t[0].validated, Validation::Link);
        // The same number in the text has its check digit confirmed.
        let both = format!("{html}<p>Tracking number: 1Z999AA10123456784</p>");
        assert_eq!(found_html("a@shop.example", Some(&both), "")[0].validated, Validation::CheckDigit);
        let rm = r#"<a href="https://www.royalmail.com/track-your-item#/tracking-results/AB123456785GB">Track</a>"#;
        assert_eq!(numbers(&found_html("a@shop.example", Some(rm), "")), [(Carrier::RoyalMail, "AB123456785GB")]);
        // A look-alike host is not a carrier.
        let fake = r#"<a href="https://ups.com.track.example/track?tracknum=1Z999AA10123456785">Track</a>"#;
        assert!(found_html("a@shop.example", Some(fake), "").is_empty());
    }

    #[test]
    fn tracking_urls_come_from_fixed_templates() {
        let cases = [
            (Carrier::Ups, "1Z999AA10123456784".to_string(), "https://www.ups.com/track?tracknum="),
            (Carrier::Usps, "9200190301123456789017".into(), "https://tools.usps.com/go/TrackConfirmAction?tLabels="),
            (Carrier::FedEx, "612900344789".into(), "https://www.fedex.com/fedextrack/?trknbr="),
            (Carrier::Dhl, "3720419861".into(), "https://www.dhl.com/global-en/home/tracking/tracking-parcel.html?tracking-id="),
            (Carrier::DhlEcommerce, "GM1234567890123456".into(), "https://www.dhl.com/global-en/home/tracking/tracking-parcel.html?tracking-id="),
            (Carrier::RoyalMail, s10_with_check("AB", "12345678", "GB"), "https://www.royalmail.com/track-your-item#/tracking-results/"),
            (Carrier::CanadaPost, "CP319475029CA".into(), "https://www.canadapost-postescanada.ca/track-reperage/en#/search?searchFor="),
            (Carrier::PostNl, "3SABCD1234567".into(), "https://jouw.postnl.nl/track-and-trace/"),
            (Carrier::DeutschePost, s10_with_check("RR", "12345678", "DE"), "https://www.deutschepost.de/de/s/sendungsverfolgung.html?piececode="),
            (Carrier::LaPoste, s10_with_check("RR", "12345678", "FR"), "https://www.laposte.fr/outils/suivre-vos-envois?code="),
            (Carrier::JapanPost, "EJ517382041JP".into(), "https://trackings.post.japanpost.jp/services/srv/search/?requestNo1="),
            (Carrier::OnTrac, "C12345678901234".into(), "https://www.ontrac.com/tracking/?number="),
        ];
        for (carrier, n, prefix) in &cases {
            let url = carrier.tracking_url(n).unwrap_or_else(|| panic!("{carrier:?} {n}"));
            assert_eq!(url, format!("{prefix}{n}"));
            assert!(carrier.host().is_some_and(|h| url.contains(h)), "{url}");
        }
        assert_eq!(Carrier::Ups.tracking_url("1z999aa10123456784"), None);
        assert_eq!(Carrier::Ups.tracking_url("1Z999AA1-0123456784"), None);
        assert_eq!(Carrier::Ups.tracking_url("1Z999AA10123456784&x=1"), None);
        assert_eq!(Carrier::Ups.tracking_url("1Z999AA10123456785"), None);
        assert_eq!(Carrier::FedEx.tracking_url("612900344788"), None);
        for (carrier, n) in [
            (Carrier::AmazonLogistics, "TBA123456789012".to_string()),
            (Carrier::Postal("ZZ".into()), "AB123456785ZZ".into()),
            (Carrier::Evri, "H00AA12345678901".into()),
            (Carrier::Dpd, "12345678901234".into()),
            (Carrier::ChinaPost, s10_with_check("RR", "12345678", "CN")),
        ] {
            assert!(validate(&carrier, &n), "{carrier:?}");
            assert_eq!(carrier.tracking_url(&n), None, "{carrier:?}");
        }
    }

    #[test]
    fn long_words_do_not_use_up_the_candidates() {
        let prose = "Thanks for shopping with Brightleaf: delivery information, shipping instructions and purchase receipts follow. ".repeat(60);
        let text = format!("{prose}\nTracking number: 1Z999AA10123456784");
        let m = Message { from: EmailAddress::new("", "shipping@brightleaf.example"), subject: "Your order has shipped".into(), ..Default::default() };
        let (t, capped) = tracking(&prepare(&m, None, &text), "shipping@brightleaf.example", None);
        assert_eq!(numbers(&t), [(Carrier::Ups, "1Z999AA10123456784")]);
        assert!(!capped);
    }

    #[test]
    fn carrier_claims_are_read_from_the_name_and_subject_start() {
        assert_eq!(claimed_carrier("UPS", "Your parcel"), Some((Carrier::Ups, true)));
        assert_eq!(claimed_carrier("Parcel Desk", "USPS: delivery attempt"), Some((Carrier::Usps, false)));
        assert_eq!(claimed_carrier("Royal Mail Group", "Hi"), Some((Carrier::RoyalMail, true)));
        assert_eq!(claimed_carrier("Amazon Delivery", "Hi"), Some((Carrier::AmazonLogistics, true)));
        assert_eq!(claimed_carrier("Amazon", "Your order"), None);
        assert_eq!(claimed_carrier("Support Groups", "Meeting"), None);
        assert_eq!(claimed_carrier("Brightleaf Home", "Your Brightleaf order #BL-48213 has shipped with UPS"), None, "past the first 40 bytes");
        // Ordinary words: hyphenated, or the acronym's letters in lowercase.
        assert_eq!(claimed_carrier("Dana Lee", "Follow-ups from today's meeting"), None);
        assert_eq!(claimed_carrier("Acme Team", "New sign-ups this week"), None);
        assert_eq!(claimed_carrier("Jonas", "Ups, falscher Anhang"), None);
        assert_eq!(claimed_carrier("Dpd Fan", "Hi"), None);
        assert_eq!(claimed_carrier("Jonas", "UPS: Paket zugestellt"), Some((Carrier::Ups, false)));
        assert_eq!(carrier_sender("ups.com"), Some(Carrier::Ups));
        assert!(sends_for(&Carrier::Dhl, "dhl.de") && !sends_for(&Carrier::Ups, "ups-redelivery.example"));
    }
}
