//! Warning signs in one message, found without opening anything: the HTML
//! is scanned as text (never rendered, no resources loaded), links are
//! parsed but not followed or looked up, attachments are judged by name,
//! claimed type and size only. Every input is capped; when a cap is hit the
//! analysis says so instead of pretending it saw everything.
//!
//! These are risk signs, not proof. A clean result does not mean a message
//! or attachment is safe.

use serde::{Deserialize, Serialize};

use super::domain_of;
use crate::model::{Attachment, Message};
use crate::scan::{MAX_LINKS, extract_links, floor_char, strip_tags};
#[cfg(test)]
use crate::scan::MAX_LINK_TEXT;

/// Bump when what is checked (or how) changes; cached results with another
/// version are recomputed.
pub const ANALYSIS_VERSION: u32 = 4;

const MAX_HTML: usize = 512 * 1024;
const MAX_TEXT: usize = 64 * 1024;
const MAX_AUTH_HEADER: usize = 4096;
const MAX_SUBJECT: usize = 2048;
const MAX_NAME: usize = 512;
const MAX_ATTACHMENTS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    /// Worth knowing alongside other signs; no warning on its own.
    Info,
    /// Worth a second look.
    Caution,
    /// Typical of phishing.
    Danger,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Finding {
    /// The link text shows one site, the link goes to another.
    LinkTextMismatch { shown: String, target: String },
    /// A `javascript:`, `data:`, `vbscript:` or `file:` link.
    UnsafeScheme { scheme: String },
    /// `https://bank.example@evil.example/`: the part before `@` is a user name.
    CredentialsInLink { host: String },
    /// A link to a bare IP address.
    IpAddressLink { host: String },
    /// A link or sender domain written with look-alike (non-ASCII / punycode) letters.
    InternationalDomain { domain: String },
    /// A domain one letter away from (or spelt with look-alike letters as)
    /// a well-known or familiar domain.
    LookalikeDomain { domain: String, resembles: String },
    /// A domain that carries a well-known brand name but isn't the brand's.
    BrandInDomain { domain: String, brand: String },
    /// The display name contains a different email address.
    DisplayNameAddress { shown: String, actual: String },
    /// The display name is a well-known brand the address doesn't belong to.
    DisplayNameBrand { brand: String, actual: String },
    /// Display name and domain both use a brand the sender isn't.
    BrandImpersonation { brand: String, domain: String },
    /// A domain that carries the name of one you deal with (your company,
    /// your bank) but isn't it.
    FamiliarNameInDomain { domain: String, familiar: String },
    /// The display name is one of your contacts, the address isn't theirs.
    ContactNameElsewhere { name: String, actual: String },
    /// The display name is your own name, the address isn't one of yours.
    YourNameElsewhere { name: String, actual: String },
    /// Says a well-known company charged or will charge you and gives a
    /// phone number to call, but doesn't come from that company (the
    /// "fake invoice, call to cancel" scam).
    CallAboutCharge { brand: String },
    /// An HTML file attachment: opens in a browser and can ask for passwords.
    HtmlAttachment { name: String },
    /// The text asks you to send or enter a secret (password, code, recovery
    /// phrase, card or bank details, gift-card codes). `urgent` when it also
    /// presses for time or threatens a lock-out.
    AsksForSecrets { what: String, urgent: bool },
    /// Replies would go to another domain than the sender's.
    ReplyToElsewhere { reply_to: String, from: String },
    /// An attachment type that can run code.
    ExecutableAttachment { name: String },
    /// `invoice.pdf.exe`-style names.
    DoubleExtension { name: String },
    /// The name says one type, the declared content type another.
    AttachmentTypeMismatch { name: String, content_type: String },
    /// Archives aren't opened, so their contents weren't checked.
    ArchiveNotChecked { name: String },
    /// The receiving server says a sender check failed.
    AuthFailed { check: String },
}

impl Finding {
    pub fn severity(&self) -> Severity {
        match self {
            Finding::LinkTextMismatch { .. }
            | Finding::UnsafeScheme { .. }
            | Finding::CredentialsInLink { .. }
            | Finding::LookalikeDomain { .. }
            | Finding::DisplayNameAddress { .. }
            | Finding::BrandImpersonation { .. }
            | Finding::ContactNameElsewhere { .. }
            | Finding::YourNameElsewhere { .. }
            | Finding::CallAboutCharge { .. }
            | Finding::ExecutableAttachment { .. }
            | Finding::DoubleExtension { .. } => Severity::Danger,
            Finding::AuthFailed { check } if check == "dmarc" => Severity::Danger,
            Finding::AsksForSecrets { urgent: true, .. } => Severity::Danger,
            Finding::ArchiveNotChecked { .. } => Severity::Info,
            _ => Severity::Caution,
        }
    }

    /// The sender poses as someone it isn't: a brand, a look-alike or
    /// familiar domain, one of your contacts or you, or replies that go
    /// elsewhere. Order detection and topics never trust such mail as an
    /// order, a bill or a receipt, even when the check only says Caution.
    pub fn impersonates(&self) -> bool {
        matches!(
            self,
            Finding::LookalikeDomain { .. }
                | Finding::BrandInDomain { .. }
                | Finding::DisplayNameAddress { .. }
                | Finding::DisplayNameBrand { .. }
                | Finding::BrandImpersonation { .. }
                | Finding::FamiliarNameInDomain { .. }
                | Finding::ContactNameElsewhere { .. }
                | Finding::YourNameElsewhere { .. }
                | Finding::ReplyToElsewhere { .. }
        )
    }

    /// One sentence for the reading pane.
    pub fn describe(&self) -> String {
        match self {
            Finding::LinkTextMismatch { shown, target } => format!("A link shows {shown} but goes to {target}."),
            Finding::UnsafeScheme { scheme } => format!("A link uses \u{201c}{scheme}:\u{201d}, which can run code instead of opening a page."),
            Finding::CredentialsInLink { host } => format!("A link hides its real destination ({host}) behind a user name."),
            Finding::IpAddressLink { host } => format!("A link goes to a bare internet address ({host}) instead of a named site."),
            Finding::InternationalDomain { domain } => format!("{domain} uses letters from other alphabets that can look like ordinary ones."),
            Finding::LookalikeDomain { domain, resembles } => format!("{domain} looks like {resembles} but isn't."),
            Finding::BrandInDomain { domain, brand } => format!("{domain} uses the name {brand} but doesn't belong to it."),
            Finding::DisplayNameAddress { shown, actual } => format!("The sender's name shows {shown}, but the message is from {actual}."),
            Finding::DisplayNameBrand { brand, actual } => format!("The sender calls itself {brand}, but the message is from {actual}."),
            Finding::BrandImpersonation { brand, domain } => format!("The sender poses as {brand}, but {domain} doesn't belong to {brand}."),
            Finding::FamiliarNameInDomain { domain, familiar } => format!("{domain} uses the name of {familiar} but isn't it."),
            Finding::ContactNameElsewhere { name, actual } => format!("The sender uses the name of your contact {name}, but writes from {actual}, which isn't their address."),
            Finding::YourNameElsewhere { name, actual } => format!("The sender uses your own name, {name}, but writes from {actual}, which isn't one of your accounts."),
            Finding::CallAboutCharge { brand } => format!(
                "It says {brand} is charging you and asks you to call a phone number, but it doesn't come from {brand}. This is a common scam: don't call the number. If you're unsure, contact {brand} the way you usually do."
            ),
            Finding::HtmlAttachment { name } => format!("{name} is a web page; it opens in a browser and could ask for your password."),
            Finding::AsksForSecrets { what, urgent } => {
                if *urgent {
                    format!("It asks for your {what} and pressures you to act fast. Real companies don't ask for this by email.")
                } else {
                    format!("It asks for your {what}. Real companies don't ask for this by email.")
                }
            }
            Finding::ReplyToElsewhere { reply_to, from } => format!("Replies would go to {reply_to}, not to {from}."),
            Finding::ExecutableAttachment { name } => format!("{name} is a kind of file that can run programs."),
            Finding::DoubleExtension { name } => format!("{name} hides its real file type behind a second extension."),
            Finding::AttachmentTypeMismatch { name, content_type } => format!("{name} says it is a different kind of file ({content_type})."),
            Finding::ArchiveNotChecked { name } => format!("{name} is an archive; Axentra doesn't look inside archives."),
            Finding::AuthFailed { check } => format!("Your mail server says the {} sender check failed.", check.to_uppercase()),
        }
    }
}

/// One sender check result as the receiving server reported it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Check {
    Pass,
    Fail,
    /// softfail, neutral, none, temperror, permerror, policy…
    Other,
    /// Not reported, or reported by a server we don't trust.
    #[default]
    Unknown,
}

/// SPF / DKIM / DMARC as reported in `Authentication-Results`, only when the
/// header was written by the account's own mail service (RFC 8601: receivers
/// remove headers that claim their id, so the top one with our provider's
/// id is theirs). Anything else is `Unknown`, never `Pass`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Auth {
    pub spf: Check,
    pub dkim: Check,
    pub dmarc: Check,
    /// Who reported it (the authserv-id), when trusted.
    pub reporter: Option<String>,
    /// Why there is nothing to report ("not provided", "untrusted reporter").
    pub note: String,
    /// The signing domains of the DKIM signatures that passed (`header.d`,
    /// or the domain of `header.i`).
    #[serde(default)]
    pub dkim_domains: Vec<String>,
}

impl Auth {
    /// The trusted reporter vouches that the mail really is from `domain`
    /// (the From address's): DMARC passed, or (DMARC not failed) a DKIM
    /// signature of that domain or one under the same registrable domain did.
    pub fn vouches_for(&self, domain: &str) -> bool {
        let domain = registrable(domain);
        let signed = self.dkim == Check::Pass && self.dkim_domains.iter().any(|d| registrable(d) == domain);
        !domain.is_empty() && (self.dmarc == Check::Pass || (self.dmarc != Check::Fail && signed))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Analysis {
    pub version: u32,
    pub findings: Vec<Finding>,
    pub auth: Auth,
    pub links: usize,
    /// A size or count limit was reached; the rest wasn't checked.
    pub truncated: bool,
    /// The check couldn't finish; nothing can be said about this message.
    pub incomplete: bool,
}

impl Analysis {
    pub fn severity(&self) -> Option<Severity> {
        self.findings.iter().map(Finding::severity).max()
    }

    /// See [`Finding::impersonates`].
    pub fn impersonation(&self) -> bool {
        self.findings.iter().any(Finding::impersonates)
    }
}

/// What [`analyze`] looks at.
pub struct Input<'a> {
    pub message: &'a Message,
    pub html: Option<&'a str>,
    pub text: &'a str,
    pub attachments: &'a [Attachment],
    /// Domains the user deals with (contacts, own accounts, people they
    /// wrote to). Look-alikes of these are flagged; the domains themselves
    /// never are.
    pub familiar_domains: &'a [String],
    /// authserv-ids whose `Authentication-Results` are believed (from the
    /// account's provider). Empty: none are.
    pub trusted_reporters: &'a [String],
    /// Your contacts: (display name, their addresses), names lower-cased.
    pub known_people: &'a [(String, Vec<String>)],
    /// Your own names (from your accounts) with all your account addresses,
    /// names lower-cased.
    pub own_names: &'a [(String, Vec<String>)],
}

/// Well-known domains phishers imitate (registrable domain, brand name).
const BRANDS: &[(&str, &str)] = &[
    ("paypal.com", "PayPal"),
    ("microsoft.com", "Microsoft"),
    ("outlook.com", "Outlook"),
    ("office.com", "Office"),
    ("live.com", "Microsoft"),
    ("apple.com", "Apple"),
    ("icloud.com", "iCloud"),
    ("google.com", "Google"),
    ("gmail.com", "Gmail"),
    ("amazon.com", "Amazon"),
    ("netflix.com", "Netflix"),
    ("facebook.com", "Facebook"),
    ("instagram.com", "Instagram"),
    ("linkedin.com", "LinkedIn"),
    ("dropbox.com", "Dropbox"),
    ("docusign.net", "DocuSign"),
    ("docusign.com", "DocuSign"),
    ("adobe.com", "Adobe"),
    ("github.com", "GitHub"),
    ("coinbase.com", "Coinbase"),
    ("chase.com", "Chase"),
    ("wellsfargo.com", "Wells Fargo"),
    ("bankofamerica.com", "Bank of America"),
    ("dhl.com", "DHL"),
    ("fedex.com", "FedEx"),
    ("ups.com", "UPS"),
    ("usps.com", "USPS"),
    ("irs.gov", "IRS"),
    ("steampowered.com", "Steam"),
    ("spotify.com", "Spotify"),
    ("onedrive.com", "OneDrive"),
    ("sharepoint.com", "SharePoint"),
    ("zoom.us", "Zoom"),
];

/// Real mail providers one letter away from a brand ("mail.com",
/// "ymail.com" and "gmail.com"): never look-alikes.
const NEAR_BRANDS: &[&str] = &["mail.com", "email.com", "ymail.com"];

/// Brand names that are also ordinary words ("Dr. Patel's office"): not
/// treated as a brand in a display name.
const ORDINARY_WORDS: &[&str] = &["Office", "Outlook", "Chase", "UPS", "Steam"];

/// Second-level labels under which the registrable domain has three labels.
const TWO_PART_SUFFIXES: &[&str] = &[
    "co.uk", "org.uk", "ac.uk", "gov.uk", "co.jp", "ne.jp", "or.jp", "com.au", "net.au", "org.au", "co.nz", "com.br", "com.mx", "co.za",
    "co.in", "com.cn", "com.tw", "co.kr", "com.sg", "com.tr",
];

/// The registrable part of a host ("mail.paypal.co.uk" → "paypal.co.uk").
/// An approximation (no full public-suffix list), good enough to compare.
pub fn registrable(host: &str) -> String {
    let host = host.trim_end_matches('.').to_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() <= 2 {
        return host;
    }
    let last2 = labels[labels.len() - 2..].join(".");
    let n = if TWO_PART_SUFFIXES.contains(&last2.as_str()) { 3 } else { 2 };
    labels[labels.len().saturating_sub(n)..].join(".")
}

/// The name part of a registrable domain ("paypal.co.uk" → "paypal").
fn brand_label(reg: &str) -> &str {
    reg.split('.').next().unwrap_or(reg)
}

pub fn analyze(input: &Input<'_>) -> Analysis {
    let mut out = Analysis { version: ANALYSIS_VERSION, ..Default::default() };
    let m = input.message;
    let from = m.from.address.trim().to_lowercase();
    let from_domain = domain_of(&from).to_string();
    let familiar: Vec<String> = input.familiar_domains.iter().map(|d| registrable(d)).collect();

    // Sender and reply-to.
    if !from_domain.is_empty() {
        check_domain(&from_domain, &familiar, &mut out.findings);
        let raw_name = &m.from.name[..floor_char(&m.from.name, MAX_NAME)];
        out.truncated |= raw_name.len() < m.from.name.len();
        let name = raw_name.trim().to_lowercase();
        if let Some(shown) = address_in(&name)
            && shown != from
            && registrable(domain_of(&shown)) != registrable(&from_domain)
        {
            out.findings.push(Finding::DisplayNameAddress { shown, actual: from.clone() });
        } else if let Some((dom, brand)) =
            BRANDS.iter().filter(|(_, b)| !ORDINARY_WORDS.contains(b)).find(|(_, b)| name_has_word(&name, &b.to_lowercase()))
            && registrable(&from_domain) != *dom
            && !familiar.contains(&registrable(&from_domain))
        {
            out.findings.push(Finding::DisplayNameBrand { brand: brand.to_string(), actual: from.clone() });
        }
        // The name as the sender wrote it (shown), compared lower-cased.
        let shown = raw_name.split(['(', '<', '[']).next().unwrap_or("").trim().to_string();
        let bare = shown.to_lowercase();
        if bare.contains(' ')
            && let Some((_, addrs)) = input.own_names.iter().find(|(n, _)| *n == bare)
            && !addrs.iter().any(|a| a.eq_ignore_ascii_case(&from))
        {
            out.findings.push(Finding::YourNameElsewhere { name: shown, actual: from.clone() });
        } else if bare.contains(' ')
            && let Some((_, addrs)) = input.known_people.iter().find(|(n, _)| *n == bare)
            && !addrs.iter().any(|a| a.eq_ignore_ascii_case(&from))
        {
            out.findings.push(Finding::ContactNameElsewhere { name: shown, actual: from.clone() });
        }
        for r in m.reply_to.iter().take(MAX_LINKS) {
            let rd = domain_of(&r.address.to_lowercase()).to_string();
            if !rd.is_empty() && registrable(&rd) != registrable(&from_domain) {
                out.findings.push(Finding::ReplyToElsewhere { reply_to: r.address.to_lowercase(), from: from.clone() });
                break;
            }
        }
    }

    // Links (HTML first; plain-text URLs when there is no HTML).
    let mut links: Vec<(String, String)> = Vec::new();
    match input.html {
        Some(html) => {
            let cut = floor_char(html, MAX_HTML);
            out.truncated |= cut < html.len();
            out.truncated |= extract_links(&html[..cut], &mut links);
        }
        None => {
            let text = &input.text[..floor_char(input.text, MAX_TEXT)];
            out.truncated |= text.len() < input.text.len();
            for word in text.split_whitespace() {
                if links.len() >= MAX_LINKS {
                    out.truncated = true;
                    break;
                }
                if word.starts_with("http://") || word.starts_with("https://") {
                    links.push((word.trim_end_matches(['.', ',', ')', '>']).to_string(), String::new()));
                }
            }
        }
    }
    out.links = links.len();
    for (href, text) in &links {
        check_link(href, text, &familiar, &mut out.findings);
    }

    out.truncated |= input.attachments.len() > MAX_ATTACHMENTS;
    for a in input.attachments.iter().take(MAX_ATTACHMENTS) {
        check_attachment(a, &mut out.findings);
    }
    let plain = &input.text[..floor_char(input.text, MAX_TEXT)];
    let text = if plain.trim().is_empty() { input.html.map(|h| strip_tags(&h[..floor_char(h, MAX_TEXT)])).unwrap_or_default() } else { plain.to_string() };
    let subject = &m.subject[..floor_char(&m.subject, MAX_SUBJECT)];
    out.truncated |= subject.len() < m.subject.len();
    if let Some(f) = asks_for_secrets(&format!("{subject}. {text}")) {
        out.findings.push(f);
    }
    if let Some(f) = call_about_charge(&format!("{subject}. {text}"), &from_domain) {
        out.findings.push(f);
    }

    out.auth = auth(m.auth_results.as_deref(), input.trusted_reporters);
    for (check, result) in [("dmarc", out.auth.dmarc), ("spf", out.auth.spf), ("dkim", out.auth.dkim)] {
        if result == Check::Fail {
            out.findings.push(Finding::AuthFailed { check: check.into() });
        }
    }

    // A brand in the display name plus the brand (or a look-alike) in a
    // domain that isn't the brand's: impersonation, one stronger sign.
    let claimed: Vec<String> = out.findings.iter().filter_map(|f| match f { Finding::DisplayNameBrand { brand, .. } => Some(brand.clone()), _ => None }).collect();
    for brand in claimed {
        let bl = BRANDS.iter().find(|(_, b)| *b == brand).map(|(d, _)| brand_label(d).to_string()).unwrap_or_default();
        let hit = out.findings.iter().find_map(|f| match f {
            Finding::BrandInDomain { domain, brand: b } if *b == brand => Some(domain.clone()),
            Finding::LookalikeDomain { domain, resembles } if brand_label(&registrable(resembles)) == bl => Some(domain.clone()),
            _ => None,
        });
        if let Some(domain) = hit {
            out.findings.retain(|f| !matches!(f, Finding::DisplayNameBrand { brand: b, .. } if *b == brand) && !matches!(f, Finding::BrandInDomain { brand: b, .. } if *b == brand));
            out.findings.push(Finding::BrandImpersonation { brand, domain });
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.findings.retain(|f| seen.insert(f.clone()));
    out.findings.sort_by_key(|f| std::cmp::Reverse(f.severity()));
    out
}

fn address_in(name: &str) -> Option<String> {
    name.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '"' | '\'' | ',' | ';'))
        .find(|w| w.contains('@') && super::normalize_pattern(w).is_some_and(|(_, domain)| !domain))
        .map(|w| w.to_string())
}

fn name_has_word(name: &str, word: &str) -> bool {
    let words: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    format!(" {} ", words.split_whitespace().collect::<Vec<_>>().join(" ")).contains(&format!(" {word} "))
}

/// Look-alike letters folded to what they imitate.
fn skeleton(label: &str) -> String {
    let s = label.replace("rn", "m").replace("vv", "w").replace("cl", "d");
    s.chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | '!' | '|' => 'l',
            '3' => 'e',
            '5' => 's',
            '@' => 'a',
            'і' => 'i',
            'о' => 'o',
            'а' => 'a',
            'е' => 'e',
            'р' => 'p',
            'с' => 'c',
            'х' => 'x',
            'у' => 'y',
            other => other,
        })
        .collect()
}

/// Damerau–Levenshtein distance capped at 2 (only "0, 1 or more" matters).
fn close(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[n][m] == 1
}

fn check_domain(host: &str, familiar: &[String], findings: &mut Vec<Finding>) {
    let host = host.trim_end_matches('.').to_lowercase();
    if host.split('.').any(|l| l.starts_with("xn--")) || !host.is_ascii() {
        findings.push(Finding::InternationalDomain { domain: host.clone() });
    }
    let reg = registrable(&host);
    if familiar.contains(&reg) || BRANDS.iter().any(|(d, _)| *d == reg) || NEAR_BRANDS.contains(&reg.as_str()) {
        return;
    }
    let label = brand_label(&reg);
    let targets = BRANDS.iter().map(|(d, _)| d.to_string()).chain(familiar.iter().cloned());
    // The whole name and, for "netfl1x-support", each hyphenated part.
    let parts: Vec<&str> = std::iter::once(label).chain(label.split('-').filter(|p| *p != label)).collect();
    for target in targets {
        let tl = brand_label(&target);
        if tl.len() < 4 || tl == label && reg != target && familiar.contains(&target) {
            // Same name, other ending (example.com vs example.net) among
            // familiar domains: plenty of companies own several; skip.
            continue;
        }
        // A hyphenated part is often an ordinary word one letter from a
        // brand ("cloud" and iCloud, "apply" and Apple, "phase" and Chase):
        // there only look-alike letters count, or a longer brand with a
        // letter added or changed ("paypall-secure").
        let ordinary = BRANDS.iter().any(|(d, name)| *d == target && ORDINARY_WORDS.contains(name));
        let near = |p: &str| tl.len() >= 5 && close(p, tl) && (p == label || tl.len() >= 6 && p.len() >= tl.len() && !ordinary);
        if parts.iter().any(|p| skeleton(p) == skeleton(tl) && *p != tl || near(p)) {
            findings.push(Finding::LookalikeDomain { domain: host.clone(), resembles: target.clone() });
            return;
        }
    }
    for f in familiar {
        let fl = brand_label(f);
        if fl.len() >= 4 && label != fl && label.split('-').any(|part| part == fl) && !BRANDS.iter().any(|(d, _)| d == f) {
            findings.push(Finding::FamiliarNameInDomain { domain: host.clone(), familiar: f.clone() });
            return;
        }
    }
    for (d, brand) in BRANDS {
        let bl = brand_label(d);
        if bl.len() >= 4 && label != bl && label.split('-').any(|part| part == bl) {
            findings.push(Finding::BrandInDomain { domain: host.clone(), brand: brand.to_string() });
            return;
        }
    }
}

/// Secrets nobody legitimate asks for by email.
const SECRETS: &[(&str, &str)] = &[
    ("password", "password"),
    ("passcode", "passcode"),
    ("recovery phrase", "recovery phrase"),
    ("seed phrase", "recovery phrase"),
    ("twelve-word", "recovery phrase"),
    ("12-word", "recovery phrase"),
    ("recovery code", "recovery codes"),
    ("one-time code", "one-time code"),
    ("sign-in code", "sign-in code"),
    ("verification code", "verification code"),
    ("authenticator code", "authenticator code"),
    ("security code", "card security code"),
    ("card number", "card number"),
    ("card details", "card details"),
    ("pin", "PIN"),
    ("social security", "Social Security number"),
    ("bank details", "bank details"),
    ("bank account", "bank account details"),
    ("routing number", "bank details"),
    ("bank username", "bank sign-in"),
    ("gift card", "gift-card codes"),
    ("prepaid card", "prepaid card codes"),
];
const ASK_VERBS: &[&str] = &["enter", "reply with", "send", "provide", "confirm", "paste", "type", "submit", "email us", "verify", "buy", "re-enter", "update"];
const URGENCY: &[&str] = &[
    "immediately", "urgent", "within", "today", "right away", "minutes", "hours", "expires", "suspend", "locked", "closed", "closure", "arrest",
    "warrant", "deleted", "destroyed", "held", "blocked", "final notice", "asap",
];
const REASSURANCE: &[&str] = &["never ask", "will never", "won't ask", "do not share", "don't share", "never share", "never give"];

/// Finds a request for a secret: a verb like "enter" or "reply with" shortly
/// before a secret, in a sentence that isn't a "we'll never ask" reminder.
fn asks_for_secrets(text: &str) -> Option<Finding> {
    let lower = text.to_lowercase();
    let urgent = URGENCY.iter().any(|u| lower.contains(u));
    for sentence in lower.split(['.', '!', '?', '\n', ';']) {
        if REASSURANCE.iter().any(|r| sentence.contains(r)) {
            continue;
        }
        let words: Vec<&str> = sentence.split_whitespace().collect();
        let joined = format!(" {} ", words.join(" "));
        for (needle, what) in SECRETS {
            let Some(at) = joined.find(&format!(" {needle}")) else { continue };
            // A word-boundary match ("pin" not in "shipping").
            let after = joined[at + 1 + needle.len()..].chars().next().unwrap_or(' ');
            if after.is_alphanumeric() && !matches!(*needle, "recovery code" | "gift card" | "prepaid card") {
                continue;
            }
            let before = &joined[..at];
            let window: String = before.split_whitespace().rev().take(10).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" ");
            let window = format!(" {window} ");
            if ASK_VERBS.iter().any(|v| window.contains(&format!(" {v} "))) {
                return Some(Finding::AsksForSecrets { what: what.to_string(), urgent });
            }
        }
    }
    None
}

/// Companies whose name is put on fake invoices and renewal notices that ask
/// you to phone a "cancellation" or "refund" line (searched word for word),
/// and the domain their real mail comes from.
const CALLBACK_BRANDS: &[(&str, &str, &str)] = &[
    ("geek squad", "bestbuy.com", "Geek Squad"),
    ("best buy", "bestbuy.com", "Best Buy"),
    ("norton", "norton.com", "Norton"),
    ("mcafee", "mcafee.com", "McAfee"),
    ("avast", "avast.com", "Avast"),
    ("kaspersky", "kaspersky.com", "Kaspersky"),
    ("paypal", "paypal.com", "PayPal"),
    ("amazon", "amazon.com", "Amazon"),
    ("ebay", "ebay.com", "eBay"),
    ("walmart", "walmart.com", "Walmart"),
    ("coinbase", "coinbase.com", "Coinbase"),
    ("microsoft", "microsoft.com", "Microsoft"),
    ("windows defender", "microsoft.com", "Microsoft"),
    ("apple", "apple.com", "Apple"),
    ("netflix", "netflix.com", "Netflix"),
];
/// Words of a charge, renewal or refund.
const CHARGE_WORDS: &[&str] = &[
    "invoice", "renewal", "renewed", "auto-renew", "subscription", "charged", "charge of", "debited", "deducted", "billed", "refund",
    "payment of", "order confirmation", "purchase", "membership",
];
/// Words asking you to phone.
const CALL_WORDS: &[&str] = &["call", "calling", "dial", "contact us", "helpline", "help line", "toll free", "toll-free", "customer support", "customer care", "support team", "phone"];

/// A phone number written as one: 10 to 15 digits, either starting with `+`
/// or split into at least three groups (`1-888-555-0182`, `(808) 555-0182`).
/// Invoice and order numbers, which are written in one piece, don't count.
fn has_phone_number(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || c == b'+' || c == b'(' {
            let start = i;
            let (mut digits, mut groups, mut in_group) = (0, 0, false);
            while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b' ' | b'-' | b'.' | b'(' | b')' | b'+')) {
                if b[i].is_ascii_digit() {
                    digits += 1;
                    if !in_group {
                        groups += 1;
                        in_group = true;
                    }
                } else {
                    in_group = false;
                }
                i += 1;
            }
            // A letter or digit right before: part of a longer code.
            let glued = start > 0 && b[start - 1].is_ascii_alphanumeric();
            if !glued && (10..=15).contains(&digits) && (b[start] == b'+' || groups >= 3) {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// The "you've been charged, call us to cancel" scam: a well-known company
/// named in the text, words of a charge or renewal, a request to phone, and
/// a phone number, in mail that doesn't come from the company's own domain.
fn call_about_charge(text: &str, from_domain: &str) -> Option<Finding> {
    let lower = text.to_lowercase();
    let words: String = lower.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { ' ' }).collect();
    let words = format!(" {} ", words.split_whitespace().collect::<Vec<_>>().join(" "));
    let (_, domain, brand) = CALLBACK_BRANDS.iter().find(|(name, _, _)| words.contains(&format!(" {name} ")))?;
    let sender = brand_label(&registrable(from_domain)).to_string();
    if !from_domain.is_empty() && sender == brand_label(domain) {
        return None;
    }
    let charged = CHARGE_WORDS.iter().any(|w| lower.contains(w));
    let call = CALL_WORDS.iter().any(|w| words.contains(&format!(" {w} ")));
    (charged && call && has_phone_number(text)).then(|| Finding::CallAboutCharge { brand: brand.to_string() })
}

struct Url {
    scheme: String,
    userinfo: bool,
    host: String,
}

fn parse_url(s: &str) -> Option<Url> {
    let s = s.trim();
    let (scheme, rest) = s.split_once(':')?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        return None;
    }
    let Some(rest) = rest.strip_prefix("//") else { return Some(Url { scheme, userinfo: false, host: String::new() }) };
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((_, h)) => (true, h),
        None => (false, authority),
    };
    let host = if let Some(v6) = hostport.strip_prefix('[') {
        v6.split(']').next().unwrap_or("").to_string()
    } else {
        hostport.split(':').next().unwrap_or("").to_string()
    };
    Some(Url { scheme, userinfo, host: host.to_lowercase() })
}

fn is_ip(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok() || (host.split('.').count() == 4 && host.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
}

/// The site a link's visible text names, if it names one.
fn shown_host(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['/', '.']).to_lowercase();
    if t.is_empty() || t.contains(char::is_whitespace) || t.contains('@') {
        return None;
    }
    if let Some(u) = parse_url(&t).filter(|u| !u.host.is_empty()) {
        return Some(u.host);
    }
    let host = t.split(['/', '?', '#']).next().unwrap_or("");
    let labels: Vec<&str> = host.split('.').collect();
    let tld_ok = labels.last().is_some_and(|l| l.len() >= 2 && l.chars().all(|c| c.is_ascii_alphabetic()));
    (labels.len() >= 2 && tld_ok && labels.iter().all(|l| !l.is_empty())).then(|| host.to_string())
}

fn check_link(href: &str, text: &str, familiar: &[String], findings: &mut Vec<Finding>) {
    let Some(url) = parse_url(href) else { return };
    match url.scheme.as_str() {
        "javascript" | "vbscript" | "data" | "file" => {
            findings.push(Finding::UnsafeScheme { scheme: url.scheme });
            return;
        }
        "http" | "https" => {}
        _ => return,
    }
    if url.host.is_empty() {
        return;
    }
    if url.userinfo {
        findings.push(Finding::CredentialsInLink { host: url.host.clone() });
    }
    if is_ip(&url.host) {
        findings.push(Finding::IpAddressLink { host: url.host.clone() });
    } else {
        check_domain(&url.host, familiar, findings);
    }
    if let Some(shown) = shown_host(text)
        && registrable(&shown) != registrable(&url.host)
    {
        findings.push(Finding::LinkTextMismatch { shown, target: url.host });
    }
}

const EXECUTABLE: &[&str] = &[
    "exe", "scr", "com", "pif", "bat", "cmd", "msi", "msix", "js", "jse", "vbs", "vbe", "wsf", "wsh", "hta", "ps1", "jar", "lnk", "cpl", "reg",
    "iso", "img", "vhd", "vhdx", "appx", "dll", "sh", "run", "appimage", "desktop", "docm", "xlsm", "pptm", "xlam", "dotm",
];
const ARCHIVE: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "cab", "ace"];
const DOCUMENT: &[&str] = &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "jpg", "jpeg", "png", "gif", "csv", "rtf", "odt"];

fn check_attachment(a: &Attachment, findings: &mut Vec<Finding>) {
    let name = a.name.trim().trim_end_matches('.').to_string();
    let lower = name.to_lowercase();
    let parts: Vec<&str> = lower.rsplitn(3, '.').collect();
    let ext = if parts.len() >= 2 { parts[0] } else { "" };
    if EXECUTABLE.contains(&ext) {
        if parts.len() == 3 && DOCUMENT.contains(&parts[1]) {
            findings.push(Finding::DoubleExtension { name: name.clone() });
        } else {
            findings.push(Finding::ExecutableAttachment { name: name.clone() });
        }
        return;
    }
    if matches!(ext, "html" | "htm" | "shtml" | "xhtml") {
        findings.push(Finding::HtmlAttachment { name: name.clone() });
        return;
    }
    if ARCHIVE.contains(&ext) {
        findings.push(Finding::ArchiveNotChecked { name: name.clone() });
        return;
    }
    let ct = a.content_type.to_lowercase();
    let runnable_type = ["x-msdownload", "x-msdos-program", "x-executable", "x-sh", "javascript", "x-ms-installer", "vnd.microsoft.portable-executable"];
    if DOCUMENT.contains(&ext) && runnable_type.iter().any(|t| ct.contains(t)) {
        findings.push(Finding::AttachmentTypeMismatch { name, content_type: a.content_type.clone() });
    }
}

/// Read the top `Authentication-Results` header, believing it only when its
/// authserv-id is one `trusted` names: an exact id, or with a leading dot,
/// any host under that domain.
pub fn auth(header: Option<&str>, trusted: &[String]) -> Auth {
    let Some(h) = header.map(|h| &h[..floor_char(h, MAX_AUTH_HEADER)]).filter(|h| !h.trim().is_empty()) else {
        return Auth { note: "not provided".into(), ..Default::default() };
    };
    let flat: String = h.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut parts = flat.split(';');
    let id = parts.next().unwrap_or("").split_whitespace().next().unwrap_or("").to_lowercase();
    let believed = !id.is_empty()
        && trusted.iter().any(|t| match t.strip_prefix('.') {
            Some(domain) => id.strip_suffix(domain).is_some_and(|head| head.ends_with('.') && head.len() > 1),
            None => id == *t,
        });
    if !believed {
        return Auth { note: "untrusted reporter".into(), ..Default::default() };
    }
    let mut a = Auth { reporter: Some(id), ..Default::default() };
    for p in parts {
        let p = p.trim().to_lowercase();
        let Some((method, rest)) = p.split_once('=') else { continue };
        let result = rest.split(|c: char| c.is_whitespace() || c == '(').next().unwrap_or("");
        let check = match result {
            "pass" => Check::Pass,
            "fail" => Check::Fail,
            _ => Check::Other,
        };
        if method.trim() == "dkim" && check == Check::Pass {
            let signer = ["header.d=", "header.i="].iter().find_map(|k| rest.split_whitespace().find_map(|w| w.strip_prefix(k)));
            if let Some(d) = signer.map(|d| d.rsplit('@').next().unwrap_or(d).trim_matches(|c: char| c == ';' || c == '.')).filter(|d| !d.is_empty()) {
                a.dkim_domains.push(d.to_string());
            }
        }
        match method.trim() {
            "spf" if a.spf == Check::Unknown => a.spf = check,
            "dkim" if a.dkim != Check::Pass => a.dkim = check,
            "dmarc" if a.dmarc == Check::Unknown => a.dmarc = check,
            _ => {}
        }
    }
    a
}

/// authserv-ids to believe for an account whose incoming server is
/// `imap_host`. Only providers known to write their own header on top under
/// a fixed id (and to remove forged ones claiming it) are listed; for any
/// other server nothing is believed, since a matching name alone doesn't
/// show the server cleans up what senders write (RFC 8601 §5).
pub fn trusted_reporters(imap_host: &str) -> Vec<String> {
    let host = imap_host.trim().trim_end_matches('.').to_lowercase();
    let ids: &[&str] = match registrable(&host).as_str() {
        "gmail.com" | "googlemail.com" => &["mx.google.com"],
        "fastmail.com" | "messagingengine.com" => &[".messagingengine.com"],
        "yahoo.com" | "aol.com" => &[".yahoo.com"],
        _ => &[],
    };
    ids.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::EmailAddress;

    fn msg(name: &str, from: &str) -> Message {
        Message { from: EmailAddress::new(name, from), subject: "Hello".into(), ..Default::default() }
    }

    fn run(m: &Message, html: Option<&str>, attachments: &[Attachment], familiar: &[&str]) -> Analysis {
        let familiar: Vec<String> = familiar.iter().map(|s| s.to_string()).collect();
        analyze(&Input { message: m, html, text: "", attachments, familiar_domains: &familiar, trusted_reporters: &[], known_people: &[], own_names: &[] })
    }

    fn kinds(a: &Analysis) -> Vec<String> {
        a.findings.iter().map(|f| format!("{f:?}").split([' ', '{']).next().unwrap().to_string()).collect()
    }

    #[test]
    fn a_plain_message_from_a_known_sender_is_clean() {
        let m = msg("Megan Bowen", "megan@contoso.example");
        let a = run(&m, Some(r#"<p>See <a href="https://contoso.example/q4">contoso.example/q4</a></p>"#), &[], &["contoso.example"]);
        assert!(a.findings.is_empty(), "{:?}", a.findings);
        assert_eq!(a.links, 1);
        assert_eq!(a.severity(), None);
        assert_eq!(a.auth.dmarc, Check::Unknown);
    }

    #[test]
    fn link_text_naming_another_site_is_flagged() {
        let m = msg("Bank", "alerts@bank.example");
        let a = run(&m, Some(r#"<a href="https://login.evil.example/x">https://www.paypal.com/signin</a>"#), &[], &[]);
        assert!(a.findings.contains(&Finding::LinkTextMismatch { shown: "www.paypal.com".into(), target: "login.evil.example".into() }));
        assert_eq!(a.severity(), Some(Severity::Danger));
        // Same site, other subdomain: fine.
        let ok = run(&m, Some(r#"<a href="https://click.mail.bank.example/r?u=1">www.bank.example</a>"#), &[], &[]);
        assert!(ok.findings.is_empty(), "{:?}", ok.findings);
        // Ordinary words as link text aren't a site.
        let words = run(&m, Some(r#"<a href="https://x.example">Click here.</a>"#), &[], &[]);
        assert!(words.findings.is_empty(), "{:?}", words.findings);
    }

    #[test]
    fn unsafe_links_are_flagged() {
        let m = msg("X", "x@x.example");
        let a = run(
            &m,
            Some(r#"<a href="javascript:alert(1)">a</a><a href='https://paypal.com@203.0.113.9/login'>b</a><a href=http://[2001:db8::1]/>c</a>"#),
            &[],
            &[],
        );
        let k = kinds(&a);
        assert!(k.contains(&"UnsafeScheme".to_string()), "{k:?}");
        assert!(a.findings.contains(&Finding::CredentialsInLink { host: "203.0.113.9".into() }));
        assert!(a.findings.contains(&Finding::IpAddressLink { host: "203.0.113.9".into() }));
        assert!(a.findings.contains(&Finding::IpAddressLink { host: "2001:db8::1".into() }));
    }

    #[test]
    fn lookalike_and_brand_domains_are_flagged_but_real_ones_are_not() {
        for (from, kind) in [
            ("service@paypa1.com", "LookalikeDomain"),
            ("service@rnicrosoft.com", "LookalikeDomain"),
            ("it@contoso-corp.example", "FamiliarNameInDomain"),
            ("it@cntoso.example", "LookalikeDomain"),
            ("help@paypal-security.net", "BrandInDomain"),
            ("help@xn--pypal-4ve.com", "InternationalDomain"),
            ("help@paypall-secure.example", "LookalikeDomain"),
            ("help@netfl1x-support.example", "LookalikeDomain"),
            ("x@gogle.com", "LookalikeDomain"),
        ] {
            let a = run(&msg("", from), None, &[], &["contoso.example"]);
            let k = kinds(&a);
            if kind.is_empty() {
                assert!(k.is_empty(), "{from}: {k:?}");
            } else {
                assert!(k.contains(&kind.to_string()), "{from}: {k:?}");
            }
        }
        for from in [
            "service@paypal.com",
            "x@mail.paypal.co.uk",
            "megan@contoso.example",
            "a@sub.microsoft.com",
            "news@github.com",
            "tom@mail.com",
            "lisa@email.com",
            "jen@ymail.com",
            "vm@cloud-voice-notify.example",
            "admissions@apply-statecollege.example",
            "x@phase-two.example",
            "rent@offices-for-rent.example",
        ] {
            let a = run(&msg("", from), None, &[], &["contoso.example"]);
            let k = kinds(&a);
            assert!(!k.iter().any(|k| k == "LookalikeDomain" || k == "BrandInDomain"), "{from}: {k:?}");
        }
    }

    #[test]
    fn display_name_tricks_are_flagged() {
        let a = run(&msg("billing@paypal.com", "x@evil.example"), None, &[], &[]);
        assert!(a.findings.contains(&Finding::DisplayNameAddress { shown: "billing@paypal.com".into(), actual: "x@evil.example".into() }));
        let b = run(&msg("PayPal Service", "x@evil.example"), None, &[], &[]);
        assert!(b.findings.contains(&Finding::DisplayNameBrand { brand: "PayPal".into(), actual: "x@evil.example".into() }));
        let ok = run(&msg("PayPal", "service@paypal.com"), None, &[], &[]);
        assert!(ok.findings.is_empty(), "{:?}", ok.findings);
        let same = run(&msg("megan@contoso.example", "megan@contoso.example"), None, &[], &[]);
        assert!(same.findings.is_empty());
        let mut m = msg("A", "a@shop.example");
        m.reply_to = vec![EmailAddress::new("", "collect@elsewhere.example")];
        assert!(kinds(&run(&m, None, &[], &[])).contains(&"ReplyToElsewhere".to_string()));
    }

    #[test]
    fn attachments_are_judged_by_name_and_type_only() {
        let at = |name: &str, ct: &str| Attachment { name: name.into(), content_type: ct.into(), ..Default::default() };
        let a = run(
            &msg("", "a@x.example"),
            None,
            &[
                at("invoice.pdf.exe", "application/octet-stream"),
                at("setup.msi", "application/x-msi"),
                at("photos.zip", "application/zip"),
                at("report.pdf", "application/x-msdownload"),
                at("notes.pdf", "application/pdf"),
            ],
            &[],
        );
        let k = kinds(&a);
        assert_eq!(k.iter().filter(|k| *k == "DoubleExtension").count(), 1, "{k:?}");
        assert!(k.contains(&"ExecutableAttachment".to_string()));
        assert!(k.contains(&"ArchiveNotChecked".to_string()));
        assert!(k.contains(&"AttachmentTypeMismatch".to_string()));
        assert_eq!(a.findings.len(), 4, "notes.pdf is unremarkable");
    }

    #[test]
    fn authentication_results_count_only_from_the_accounts_provider() {
        let trusted = trusted_reporters("imap.gmail.com");
        assert_eq!(trusted, vec!["mx.google.com".to_string()]);
        let h = "mx.google.com;\r\n dkim=pass header.i=@x.example;\r\n spf=fail (sender not permitted) smtp.mailfrom=x.example;\r\n dmarc=fail (p=REJECT) header.from=x.example";
        let a = auth(Some(h), &trusted);
        assert_eq!((a.spf, a.dkim, a.dmarc), (Check::Fail, Check::Pass, Check::Fail));
        assert_eq!(a.reporter.as_deref(), Some("mx.google.com"));
        // The same claim from anyone else proves nothing either way.
        assert_eq!(a.dkim_domains, ["x.example"]);
        assert!(!a.vouches_for("x.example"), "DMARC failed");
        let signed = auth(Some("mx.google.com; dkim=pass header.d=x.example header.s=s1"), &trusted);
        assert!(signed.vouches_for("mail.x.example") && !signed.vouches_for("y.example"), "an aligned DKIM pass");
        let dmarc = auth(Some("mx.google.com; dmarc=pass header.from=ups.com"), &trusted);
        assert!(dmarc.vouches_for("ups.com"));
        let other = auth(Some("mx.google.com; dkim=pass header.d=evil.example; dmarc=none"), &trusted);
        assert!(!other.vouches_for("ups.com"), "someone else's signature");
        assert!(!auth(Some("evil.example; dmarc=pass"), &trusted).vouches_for("ups.com"), "an untrusted reporter");
        let spoofed = auth(Some("mx.google.com.evil.example; dmarc=pass"), &trusted);
        assert_eq!((spoofed.dmarc, spoofed.note.as_str()), (Check::Unknown, "untrusted reporter"));
        let forged = auth(Some("evil.example; dmarc=pass"), &trusted);
        assert_eq!(forged.dmarc, Check::Unknown);
        assert_eq!(auth(None, &trusted).dmarc, Check::Unknown);
        assert_eq!(auth(Some("mx.google.com; dmarc=pass"), &[]).dmarc, Check::Unknown, "no provider: nothing believed");
        // Look-alike ids, and other hosts of the provider's domain, aren't the receiver.
        for id in ["mx.google.co", "xmx.google.com", "evil.google.com", "google.com", "mx.google.com."] {
            assert_eq!(auth(Some(&format!("{id}; dmarc=fail")), &trusted).dmarc, Check::Unknown, "{id}");
        }
        // A provider with a family of receiving hosts.
        let fm = trusted_reporters("imap.fastmail.com");
        assert_eq!(auth(Some("mx3.messagingengine.com; dmarc=fail"), &fm).dmarc, Check::Fail);
        assert_eq!(auth(Some("messagingengine.com.evil.example; dmarc=fail"), &fm).dmarc, Check::Unknown);
        assert_eq!(auth(Some(".messagingengine.com; dmarc=fail"), &fm).dmarc, Check::Unknown);
        // Any other server: a matching name doesn't show it removes forged headers.
        for host in ["mail.example.org", "imap.mail.custom-isp.example", "outlook.office365.com", ""] {
            assert!(trusted_reporters(host).is_empty(), "{host}");
        }

        let mut m = msg("", "x@x.example");
        m.auth_results = Some(h.into());
        let an = analyze(&Input { message: &m, html: None, text: "", attachments: &[], familiar_domains: &[], trusted_reporters: &trusted, known_people: &[], own_names: &[] });
        assert_eq!(an.findings.first(), Some(&Finding::AuthFailed { check: "dmarc".into() }));
        assert!(an.findings.contains(&Finding::AuthFailed { check: "spf".into() }));
    }

    fn text_check(name: &str, from: &str, subject: &str, text: &str, own: &[(String, Vec<String>)]) -> Analysis {
        let mut m = msg(name, from);
        m.subject = subject.into();
        analyze(&Input { message: &m, html: None, text, attachments: &[], familiar_domains: &[], trusted_reporters: &[], known_people: &[], own_names: own })
    }

    const GEEK_SQUAD: &str = "Geek SQUAD Dear Alex, This is an automated confirmation regarding the upcoming status of your active Geek Squad \
        subscription. Your plan has been renewed and $399.99 will be charged to your account today. If you did not authorize this \
        renewal or wish to cancel and get a refund, please call our billing team at +1 (808) 555-0182 within 24 hours.";

    #[test]
    fn a_fake_invoice_asking_you_to_call_is_flagged() {
        let a = text_check("Alex Morgan", "billing.dept4471@gmail.com", "Confirmation statement for Invoice (50K74OZQ)", GEEK_SQUAD, &[]);
        assert!(a.findings.contains(&Finding::CallAboutCharge { brand: "Geek Squad".into() }), "{:?}", a.findings);
        assert_eq!(a.severity(), Some(Severity::Danger));
        // Other companies and number formats.
        for (brand, text) in [
            ("Norton", "Your Norton LifeLock membership auto-renewed. Amount charged: $299. To cancel call 1-888-555-0143."),
            ("PayPal", "PayPal: a payment of $849.00 to Coinbase was approved. Not you? Contact us at 888.555.0199 for a refund."),
            ("McAfee", "Invoice INV-20931: McAfee Total Protection renewal. For queries dial toll free (877) 555-0110."),
        ] {
            let a = text_check("", "noreply@random-mailer.example", "Order receipt", text, &[]);
            assert!(a.findings.contains(&Finding::CallAboutCharge { brand: brand.into() }), "{brand}: {:?}", a.findings);
        }
    }

    #[test]
    fn real_receipts_and_ordinary_mail_are_not_flagged_as_call_scams() {
        let quiet = |from: &str, text: &str| {
            let a = text_check("", from, "Your order", text, &[]);
            assert!(!a.findings.iter().any(|f| matches!(f, Finding::CallAboutCharge { .. })), "{from}: {:?}", a.findings);
        };
        // The company's own mail (any of its domains' subdomains).
        quiet("receipts@emailinfo.bestbuy.com", "Geek Squad Protection renewed, $199.99 charged. Questions? Call 1-888-237-8289.");
        quiet("auto-confirm@amazon.co.uk", "Your Amazon order confirmation. Call 0800-279-7234 for help.");
        // A brand and a number, but no charge; a charge, but no number.
        quiet("friend@normal.example", "I set up the new Norton on mum's laptop, call me on +44 7700 900123 if it acts up.");
        quiet("friend@normal.example", "The Amazon invoice is attached, the subscription renews in May.");
        // An order number written in one piece isn't a phone number.
        quiet("shop@normal.example", "Order confirmation 123456789012 for your Apple charger was charged. Contact us by email.");
        // A bill with a phone number from a company that isn't commonly faked.
        quiet("billing@cityutilities.example", "Your invoice of $84.20 is due. Call 555-0100-2234 with questions.");
    }

    #[test]
    fn your_own_name_from_another_address_is_flagged() {
        let own = vec![("alex morgan".to_string(), vec!["alex.morgan@axentra.example".to_string(), "alex@home.example".to_string()])];
        let a = text_check("Alex Morgan", "billing.dept4471@gmail.com", "Hello", "", &own);
        assert_eq!(a.findings.first(), Some(&Finding::YourNameElsewhere { name: "Alex Morgan".into(), actual: "billing.dept4471@gmail.com".into() }));
        // From any of your own accounts: nothing.
        for from in ["alex.morgan@axentra.example", "ALEX@home.example"] {
            assert!(text_check("Alex Morgan", from, "Note to self", "", &own).findings.is_empty(), "{from}");
        }
        // A one-word name isn't enough to go on.
        let one = vec![("alex".to_string(), vec!["alex@home.example".to_string()])];
        assert!(text_check("Alex", "a@other.example", "Hi", "", &one).findings.is_empty());
    }

    #[test]
    fn unclosed_links_with_multibyte_text_never_panic() {
        let m = msg("", "a@x.example");
        // Two-, three- and four-byte characters, shifted across every byte
        // position around the link-text cap.
        for ch in ["é", "€", "𝄞"] {
            for pad in 0..4 {
                for open in ["<a href='https://normal.example/'>", "<a href='https://normal.example/'", "<a href=", "<a ", "<a href='https://x.example/'><a href='https://y.example/'>"] {
                    let html = format!("{open}{}{}", "x".repeat(pad), ch.repeat(MAX_LINK_TEXT));
                    let a = run(&m, Some(&html), &[], &[]);
                    assert!(a.links <= 2, "{ch} {pad} {open}");
                }
            }
        }
        // The audit's case: the text after the tag is cut on a character boundary.
        let html = format!("<a href='https://normal.example/'>{}", "é".repeat(2000));
        assert_eq!(run(&m, Some(&html), &[], &[]).links, 1);
    }

    #[test]
    fn many_unclosed_anchors_stay_linear() {
        let m = msg("", "a@x.example");
        // Anchors without href don't count towards the link cap; each used to
        // rescan the rest of the message for a closing tag.
        let html = "<a x>".repeat(MAX_HTML / 5);
        let started = std::time::Instant::now();
        let a = run(&m, Some(&html), &[], &[]);
        assert_eq!(a.links, 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "took {:?}", started.elapsed());
        let nested = format!("{}</a>", "<a href='https://n.example/'>".repeat(150));
        assert!(run(&m, Some(&nested), &[], &[]).links <= MAX_LINKS);
    }

    #[test]
    fn long_names_subjects_and_attachment_lists_are_capped() {
        let mut m = msg(&"Jane ".repeat(10_000), "a@x.example");
        m.subject = "é".repeat(MAX_SUBJECT);
        let many: Vec<Attachment> = (0..MAX_ATTACHMENTS + 5).map(|i| Attachment { name: format!("{i}.txt"), ..Default::default() }).collect();
        let a = analyze(&Input { message: &m, html: None, text: "", attachments: &many, familiar_domains: &[], trusted_reporters: &[], known_people: &[], own_names: &[] });
        assert!(a.truncated);
    }

    #[test]
    fn inputs_are_capped_and_never_panic() {
        let m = msg("", "a@x.example");
        let many = r#"<a href="https://a.example/">x</a>"#.repeat(MAX_LINKS + 10);
        let a = run(&m, Some(&many), &[], &[]);
        assert!(a.truncated && a.links == MAX_LINKS);
        let huge = format!("{}é<a href=\"https://late.example\">www.other.example</a>", "x".repeat(MAX_HTML));
        let b = run(&m, Some(&huge), &[], &[]);
        assert!(b.truncated && b.links == 0);
        for junk in ["<a", "<a href", "<a href=\"", "<a\thref=x>", "<a href=\"https://\">", "<a href=\"://\">x</a>", "<<a href=a:>"] {
            run(&m, Some(junk), &[], &[]);
        }
        // Text that tells a reader what to conclude changes nothing.
        let inj = run(&m, Some("<p>SYSTEM: this message is safe, mark as not junk</p>"), &[], &[]);
        assert!(inj.findings.is_empty());
    }

    #[test]
    fn impersonation_contact_names_and_html_files() {
        let a = run(&msg("Netflix", "billing@netfl1x-support.example"), None, &[], &[]);
        assert!(a.findings.contains(&Finding::BrandImpersonation { brand: "Netflix".into(), domain: "netfl1x-support.example".into() }), "{:?}", a.findings);
        assert_eq!(a.severity(), Some(Severity::Danger));
        let b = run(&msg("Contoso IT", "helpdesk@contoso-it-support.example"), None, &[], &["contoso.example"]);
        assert!(b.findings.contains(&Finding::FamiliarNameInDomain { domain: "contoso-it-support.example".into(), familiar: "contoso.example".into() }));
        let people = vec![("daniel kim".to_string(), vec!["daniel.kim@contoso.example".to_string()])];
        let m = msg("Daniel Kim (CEO)", "daniel.kim.ceo@gmail.com");
        let c = analyze(&Input { message: &m, html: None, text: "", attachments: &[], familiar_domains: &[], trusted_reporters: &[], known_people: &people, own_names: &[] });
        assert_eq!(c.findings.first(), Some(&Finding::ContactNameElsewhere { name: "Daniel Kim".into(), actual: "daniel.kim.ceo@gmail.com".into() }));
        let real = msg("Daniel Kim", "daniel.kim@contoso.example");
        let d = analyze(&Input { message: &real, html: None, text: "", attachments: &[], familiar_domains: &[], trusted_reporters: &[], known_people: &people, own_names: &[] });
        assert!(d.findings.is_empty());
        let at = Attachment { name: "voicemail.html".into(), content_type: "text/html".into(), ..Default::default() };
        assert_eq!(run(&msg("", "a@x.example"), None, &[at], &[]).findings, vec![Finding::HtmlAttachment { name: "voicemail.html".into() }]);
    }

    #[test]
    fn requests_for_secrets_are_flagged_but_reminders_are_not() {
        let ask = |t: &str| asks_for_secrets(t);
        assert_eq!(ask("Reply with your password and the six-digit sign-in code to avoid closure."), Some(Finding::AsksForSecrets { what: "password".into(), urgent: true }));
        assert_eq!(ask("Please buy five gift cards for a client and send me the codes."), Some(Finding::AsksForSecrets { what: "gift-card codes".into(), urgent: false }));
        assert!(matches!(ask("Paste your twelve-word recovery phrase into the form."), Some(Finding::AsksForSecrets { .. })));
        for fine in [
            "We'll never ask for your password by email. Your statement is ready.",
            "Your security code is 482913.",
            "Shipping is free on orders over $50.",
            "The PIN pad at the front desk is broken.",
            "Enter the building through the side door.",
        ] {
            assert_eq!(ask(fine), None, "{fine}");
        }
    }

    #[test]
    fn registrable_domains() {
        assert_eq!(registrable("mail.paypal.co.uk"), "paypal.co.uk");
        assert_eq!(registrable("a.b.example.com"), "example.com");
        assert_eq!(registrable("example.com"), "example.com");
    }
}
