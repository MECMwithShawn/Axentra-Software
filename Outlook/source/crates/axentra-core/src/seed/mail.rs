//! Demo mail: a realistic Inbox around `now` (threads, a meeting invitation,
//! newsletters in Other, flags, pins, categories, attachments), content in
//! every well-known folder, and a few hundred older messages generated
//! deterministically so scrolling and date grouping look real.

use chrono::{Duration, Utc};

use super::{SeedCtx, person};
use crate::model::*;
use crate::{Result, Store};

/// One demo message. `ago_min` is minutes before `now`.
struct M {
    from: EmailAddress,
    to: Vec<EmailAddress>,
    cc: Vec<EmailAddress>,
    subject: String,
    html: String,
    ago_min: i64,
    read: bool,
    other: bool,
    flag: FlagStatus,
    /// Days from today for the flag's due date.
    due_in_days: Option<i64>,
    pinned: bool,
    importance: Importance,
    categories: Vec<&'static str>,
    /// (name, content type, size, optional downloaded content).
    attachments: Vec<(&'static str, &'static str, i64, Option<&'static [u8]>)>,
    conversation: Option<String>,
    meeting: Option<(MeetingMessageKind, EventId)>,
    snooze_hours: Option<i64>,
}

impl Default for M {
    fn default() -> Self {
        M {
            from: EmailAddress::default(),
            to: Vec::new(),
            cc: Vec::new(),
            subject: String::new(),
            html: String::new(),
            ago_min: 0,
            read: true,
            other: false,
            flag: FlagStatus::NotFlagged,
            due_in_days: None,
            pinned: false,
            importance: Importance::Normal,
            categories: Vec::new(),
            attachments: Vec::new(),
            conversation: None,
            meeting: None,
            snooze_hours: None,
        }
    }
}

fn para(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("<p>{l}</p>")).collect::<Vec<_>>().join("")
}

fn sig(name: &str) -> String {
    format!("<p>Thanks,<br>{name}</p>")
}

/// Outlook-style quoted history under a reply.
fn quoted(from: &EmailAddress, sent: &str, to: &str, subject: &str, body: &str) -> String {
    format!(
        "<hr style=\"display:inline-block;width:98%\"><div id=\"divRplyFwdMsg\" dir=\"ltr\"><b>From:</b> {} &lt;{}&gt;<br>\
         <b>Sent:</b> {sent}<br><b>To:</b> {to}<br><b>Subject:</b> {subject}</div><div><br></div><div class=\"ax-quoted\">{body}</div>",
        from.name, from.address
    )
}

fn conversation_key(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        match ["re:", "fw:", "fwd:"].iter().find(|p| lower.starts_with(*p)) {
            Some(p) => s = s[p.len()..].trim_start(),
            None => break,
        }
    }
    format!("demo-{s}")
}

struct Seeder<'a> {
    store: &'a Store,
    ctx: &'a SeedCtx,
    serial: u32,
}

impl Seeder<'_> {
    fn add(&mut self, folder: FolderId, m: M) -> Result<MessageId> {
        self.serial += 1;
        let now = self.ctx.now.with_timezone(&Utc);
        let received = now - Duration::minutes(m.ago_min);
        let text = crate::html::html_to_text(&m.html);
        let mentions_me = text.contains("@Alex");
        let today = self.ctx.now.date_naive();
        let conversation_id = m.conversation.clone().unwrap_or_else(|| conversation_key(&m.subject));
        // Replies and forwards point at the previous message of their thread.
        let lower = m.subject.to_ascii_lowercase();
        let in_reply_to = if lower.starts_with("re:") || lower.starts_with("fw:") {
            self.store
                .conversation(&conversation_id)?
                .into_iter()
                .filter(|p| p.received_at < received)
                .max_by_key(|p| p.received_at)
                .map(|p| p.internet_message_id)
        } else {
            None
        };
        let mut msg = Message {
            account_id: self.ctx.account_id,
            folder_id: folder,
            conversation_id,
            in_reply_to,
            internet_message_id: format!("demo-{}@axentra.example", self.serial),
            from: m.from.clone(),
            to: if m.to.is_empty() { vec![self.ctx.me.clone()] } else { m.to.clone() },
            cc: m.cc.clone(),
            subject: m.subject.clone(),
            preview: crate::text::preview(&text, 200),
            received_at: received,
            sent_at: Some(received - Duration::seconds(30)),
            is_read: m.read,
            flag: m.flag,
            flag_due: m.due_in_days.map(|d| today + Duration::days(d)),
            is_pinned: m.pinned,
            importance: m.importance,
            has_attachments: !m.attachments.is_empty(),
            categories: m.categories.iter().map(|c| c.to_string()).collect(),
            inference: if m.other { InferenceClass::Other } else { InferenceClass::Focused },
            mentions_me,
            meeting_kind: m.meeting.map(|(k, _)| k),
            meeting_event_id: m.meeting.map(|(_, e)| e),
            ..Default::default()
        };
        let body = MessageBody { html: Some(m.html.clone()), text, ..Default::default() };
        let id = self.store.insert_message(&mut msg, Some(&body))?;
        for (name, ct, size, data) in &m.attachments {
            let mut a = Attachment {
                message_id: id,
                name: name.to_string(),
                content_type: ct.to_string(),
                size: data.map(|d| d.len() as i64).unwrap_or(*size),
                ..Default::default()
            };
            self.store.insert_attachment(&mut a, *data)?;
        }
        if let Some(h) = m.snooze_hours {
            self.store.snooze(&[id], Some(now + Duration::hours(h)))?;
        }
        Ok(id)
    }
}

const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

const ACTUALS_CSV: &[u8] = b"Line item,Plan,Actual,Variance\nContractors,48000,41200,-6800\nCloud,22500,23140,640\nTravel,9000,8210,-790\nSoftware,12000,11950,-50\n";
const PALETTE_SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="80"><rect width="80" height="80" fill="#0f6cbd"/><rect x="80" width="80" height="80" fill="#c50f1f"/><rect x="160" width="80" height="80" fill="#107c10"/><rect x="240" width="80" height="80" fill="#fde300"/></svg>"##;
const NOTES_TXT: &[u8] = b"Customer interviews - top themes\n\n1. Faster search (5 of 5)\n2. Offline drafts (3 of 5)\n3. Shared calendars (2 of 5)\n";
const AGENDA_ICS_TXT: &[u8] = b"09:00 Welcome\n09:30 Q4 priorities\n11:00 Team breakouts\n12:30 Lunch\n";

const TOPICS: &[&str] = &[
    "onboarding", "the pricing page", "the Q3 roadmap", "the vendor renewal", "the mobile app", "search relevance",
    "data retention", "the hiring plan", "launch readiness", "the partner portal", "the support backlog", "the design system",
    "API rate limits", "the team offsite", "the security review", "the release train", "customer churn", "the analytics pipeline",
];

const SUBJECTS: &[&str] = &[
    "Weekly status: {}", "Notes from the {} sync", "Follow-up on {}", "Question about {}", "Draft agenda for {}",
    "Reminder: {} review on Thursday", "Metrics for {}", "Feedback on {}", "Timeline for {}", "Workshop on {}",
    "Decision needed: {}", "Quick update on {}",
];

const BODY_LINES: &[&str] = &[
    "Sharing a quick summary so we're all on the same page.",
    "Let me know if anything here looks off before Friday.",
    "I've added the open questions to the doc; comments welcome.",
    "We're on track overall, with two risks called out below.",
    "Could you take a look when you get a minute?",
    "No action needed, this is just for awareness.",
    "I'll set up time next week to go through the details.",
];

const NEWSLETTERS: &[(&str, &str)] = &[
    ("Northwind Weekly", "newsletter@northwind.example"),
    ("Lumen Design Digest", "digest@lumen.example"),
    ("Skyline Air", "deals@skyline.example"),
    ("Harbor Bank", "alerts@harborbank.example"),
];

/// A newsletter body with remote images (blocked until the user allows them).
fn newsletter_html(brand: &str, headline: &str, image_host: &str) -> String {
    format!(
        "<div style=\"max-width:600px;margin:0 auto;font-family:Arial,sans-serif\">\
         <p><img src=\"https://{image_host}/banner.png\" width=\"600\" height=\"160\" alt=\"{brand}\"></p>\
         <h2 style=\"color:#0f548c\">{headline}</h2>\
         <p>Here's what's new this week. Tap through for the full stories.</p>\
         <p><img src=\"https://{image_host}/story-1.jpg\" width=\"280\" alt=\"Story\"> \
         <img src=\"https://{image_host}/story-2.jpg\" width=\"280\" alt=\"Story\"></p>\
         <p><a href=\"https://{image_host}/read\">Read online</a> · <a href=\"https://{image_host}/unsubscribe\">Unsubscribe</a></p>\
         <img src=\"https://{image_host}/open.gif\" width=\"1\" height=\"1\" alt=\"\"></div>"
    )
}

pub(super) fn seed(store: &Store, ctx: &SeedCtx) -> Result<()> {
    let acct = ctx.account_id;
    let inbox = store.folder_for_role(acct, FolderRole::Inbox)?;
    let sent = store.folder_for_role(acct, FolderRole::Sent)?;
    let archive = store.folder_for_role(acct, FolderRole::Archive)?;
    let deleted = store.folder_for_role(acct, FolderRole::Deleted)?;
    let junk = store.folder_for_role(acct, FolderRole::Junk)?;
    let drafts = store.folder_for_role(acct, FolderRole::Drafts)?;
    let projects = store.create_folder(acct, None, "Projects")?;
    let launch = store.create_folder(acct, Some(projects), "Q4 Launch")?;
    let receipts = store.create_folder(acct, None, "Receipts")?;
    // Outlook on the web's default Favorites: Inbox, Sent Items, Deleted Items.
    store.set_folder_favorite(drafts, false)?;
    store.set_folder_favorite(deleted, true)?;
    store.set_folder_favorite(launch, true)?;

    let me = ctx.me.clone();
    let mut s = Seeder { store, ctx, serial: 0 };

    // ---------------------------------------------------------------- today and this week
    s.add(inbox, M {
        from: person(0),
        cc: vec![person(4)],
        subject: "Q4 launch plan — final review before Thursday".into(),
        html: para(&["Hi Alex,", "I've attached the final launch plan. Could you review the timeline on slide 4 and confirm the engineering dates before Thursday's sync?", "The marketing team needs a go/no-go by end of day Wednesday."]) + &sig("Priya"),
        ago_min: 25, read: false, flag: FlagStatus::Flagged, due_in_days: Some(0), pinned: true,
        attachments: vec![("Q4 Launch Plan.pptx", PPTX, 2_480_000, None)],
        categories: vec!["Red category"], importance: Importance::High,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(1),
        to: vec![me.clone(), person(3), person(7)],
        subject: "Code freeze moved to Friday".into(),
        html: para(&["Team,", "We're moving code freeze to Friday 5 PM so the payments fix can land. Please get reviews in by Thursday noon.", "<a href=\"mailto:alex.morgan@axentra.example\">@Alex Morgan</a> can you confirm the release notes owner?"]) + &sig("Marcus"),
        ago_min: 70, read: false,
        ..Default::default()
    })?;

    // A three-message thread: Hannah, my reply (Sent), Hannah again.
    let hannah = person(4);
    let venue = "Venue contract — deposit due Friday";
    let v1 = para(&["Hi Alex,", "The warehouse venue sent over the contract. They need a 30% deposit by Friday to hold the date. Are you OK with me signing?"]) + &sig("Hannah");
    s.add(inbox, M {
        from: hannah.clone(),
        subject: venue.into(),
        html: v1.clone(),
        ago_min: 60 * 26, read: true,
        attachments: vec![("Venue contract.pdf", "application/pdf", 312_000, None)],
        categories: vec!["Orange category"],
        ..Default::default()
    })?;
    let v2 = para(&["Hi Hannah,", "Yes, go ahead and sign. Finance already approved the budget line."]) + &sig("Alex")
        + &quoted(&hannah, "Sunday", "Alex Morgan", venue, &v1);
    s.add(sent, M {
        from: me.clone(),
        to: vec![hannah.clone()],
        subject: format!("RE: {venue}"),
        html: v2.clone(),
        ago_min: 60 * 20, read: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: hannah.clone(),
        subject: format!("RE: {venue}"),
        html: para(&["Signed and sent! Deposit invoice attached for your records. They also offered a free walkthrough next Tuesday if you want to join."]) + &sig("Hannah")
            + &quoted(&me, "Monday", "Hannah Okafor", &format!("RE: {venue}"), &v2),
        ago_min: 180, read: false,
        attachments: vec![("Deposit invoice.pdf", "application/pdf", 64_000, None)],
        categories: vec!["Orange category"],
        ..Default::default()
    })?;

    s.add(inbox, M {
        from: person(2),
        subject: "Updated brand assets".into(),
        html: format!("{}<p><img src=\"https://cdn.lumen.example/preview/brand-board.png\" width=\"480\" alt=\"Brand board\"></p>{}",
            para(&["Hi Alex,", "The updated brand board is below and the source files are in the shared folder. I've also attached the new palette."]), sig("Elena")),
        ago_min: 140, read: true,
        attachments: vec![
            ("Brand guidelines v3.pdf", "application/pdf", 5_310_000, None),
            ("Logo pack.zip", "application/zip", 18_200_000, None),
            ("Color palette.svg", "image/svg+xml", 0, Some(PALETTE_SVG)),
        ],
        categories: vec!["Purple category", "Blue category"],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Skyline Air", "no-reply@skyline.example"),
        subject: "Your trip to Seattle is confirmed".into(),
        html: para(&["Confirmation code: <b>K7Q2XP</b>", "Mon, Oct 26 · SFO 8:05 AM → SEA 10:12 AM", "Seat 14C · Main Cabin"]),
        ago_min: 200, read: false, other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(5),
        subject: "Budget variance — September actuals".into(),
        html: para(&["Alex,", "September came in 3.2% under plan, mostly from delayed contractor spend. Details in the attached workbook, and a CSV summary if you just want the numbers.", "Happy to walk through it tomorrow."]) + &sig("Wei"),
        ago_min: 300, read: true,
        attachments: vec![
            ("Sept actuals.xlsx", XLSX, 412_000, None),
            ("Sept actuals summary.csv", "text/csv", 0, Some(ACTUALS_CSV)),
        ],
        categories: vec!["Green category"],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Northwind Weekly", "newsletter@northwind.example"),
        subject: "This week at Northwind: supply chain wins and a new warehouse".into(),
        html: newsletter_html("Northwind Weekly", "Supply chain wins and a new warehouse", "cdn.northwind.example"),
        ago_min: 60 * 5, read: false, other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Lumen Design Digest", "digest@lumen.example"),
        subject: "Design Digest: 12 type pairings for product UI".into(),
        html: newsletter_html("Lumen Design Digest", "12 type pairings for product UI", "img.lumen.example"),
        ago_min: 60 * 9, read: true, other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(4),
        subject: "Campaign creative — need your sign-off".into(),
        html: para(&["Hi Alex,", "Three directions are attached. We're leaning toward option B. Can you sign off by Wednesday?"]) + &sig("Hannah"),
        ago_min: 60 * 22, read: false, flag: FlagStatus::Flagged, due_in_days: Some(2),
        attachments: vec![("Option A.png", "image/png", 820_000, None), ("Option B.png", "image/png", 790_000, None), ("Option C.png", "image/png", 805_000, None)],
        categories: vec!["Orange category"],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(7),
        subject: "Regression suite results — build 4127".into(),
        html: para(&["All 1,284 tests passed except two flaky UI tests in checkout. Re-running now.", "Dashboard: https://ci.axentra.example/build/4127"]) + &sig("Daniel"),
        ago_min: 60 * 26, read: true,
        ..Default::default()
    })?;
    // A reply in the code-freeze thread from Daniel (second Inbox message of that conversation).
    s.add(inbox, M {
        from: person(7),
        to: vec![person(1), me.clone()],
        subject: "RE: Code freeze moved to Friday".into(),
        html: para(&["Works for QA. We'll run the full regression on Thursday night so there's time for fixes."]) + &sig("Daniel"),
        ago_min: 35, read: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(8),
        subject: "NDA for Lumen partnership".into(),
        html: para(&["Alex,", "Attached is the mutual NDA with Lumen Studio. Two redlines in section 4. Please review and let me know if you're comfortable."]) + &sig("Aisha"),
        ago_min: 60 * 30, read: true, flag: FlagStatus::Complete,
        attachments: vec![("Mutual NDA - Lumen.docx", DOCX, 96_000, None)],
        ..Default::default()
    })?;

    // The interview invitation, linked to the seeded calendar event.
    if let Some(event) = store.search_events("Interview: Senior Engineer")?.into_iter().next() {
        let when = event.start.with_timezone(&chrono::Local);
        let end = event.end.with_timezone(&chrono::Local);
        s.add(inbox, M {
            from: person(9),
            subject: event.title.clone(),
            html: para(&[
                &format!("<b>When:</b> {} – {}", when.format("%A, %B %-d, %Y %-I:%M %p"), end.format("%-I:%M %p")),
                &format!("<b>Where:</b> {}", event.location),
                "Hi Alex, you're on the loop for the system design round. The candidate packet is attached; please submit feedback in the hiring tool within 24 hours.",
            ]) + &sig("Liam"),
            ago_min: 60 * 50, read: false,
            meeting: Some((MeetingMessageKind::Request, event.id)),
            attachments: vec![("Candidate packet.pdf", "application/pdf", 540_000, None)],
            ..Default::default()
        })?;
    }

    s.add(inbox, M {
        from: person(6),
        subject: "Re: Statement of work — revised pricing".into(),
        html: para(&["Hi Alex,", "Thanks for the call. Revised SOW attached with the phased pricing we discussed."]) + &sig("Sofia"),
        ago_min: 60 * 54, read: true,
        attachments: vec![("SOW - Phase 1.pdf", "application/pdf", 240_000, None)],
        categories: vec!["Blue category"],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Harbor Bank", "alerts@harborbank.example"),
        subject: "Your monthly statement is ready".into(),
        html: para(&["Your October statement is available in online banking."]),
        ago_min: 60 * 72, read: true, other: true,
        ..Default::default()
    })?;
    // Many recipients.
    s.add(inbox, M {
        from: person(10),
        to: vec![me.clone(), person(0), person(1), person(2), person(3), person(4), person(5), person(6), person(7)],
        cc: vec![person(8), person(9), person(11), EmailAddress::new("Facilities", "facilities@northwind.example")],
        subject: "All-hands: Q4 kickoff logistics".into(),
        html: para(&["Hi all,", "The Q4 kickoff is next Monday at 9:00 AM in the atrium. Breakfast from 8:30. The agenda is attached; remote folks will get a meeting link the day before.", "Please reply by Thursday if you have dietary requirements."]) + &sig("Grace"),
        ago_min: 60 * 80, read: true,
        attachments: vec![("Kickoff agenda.txt", "text/plain", 0, Some(AGENDA_ICS_TXT))],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(10),
        subject: "Office move logistics".into(),
        html: para(&["Hi all,", "Movers arrive Saturday at 8 AM. Please pack your desks by Friday 3 PM and label boxes with your name and new desk number."]) + &sig("Grace"),
        ago_min: 60 * 96, read: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Facilities", "facilities@northwind.example"),
        subject: "FYI: parking garage maintenance this weekend".into(),
        html: para(&["Level 2 of the garage will be closed Saturday and Sunday for resurfacing. Use levels 3 and 4."]),
        ago_min: 60 * 100, read: true, importance: Importance::Low,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(3),
        subject: "PR #2291: offline sync for drafts".into(),
        html: para(&["Alex — PR is up for offline draft sync. It touches the outbox queue, so I'd love your eyes on the retry logic."]) + &sig("Tomás"),
        ago_min: 60 * 120, read: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(11),
        subject: "Hotel options near the Seattle office".into(),
        html: para(&["Here are three hotels within walking distance. All have the corporate rate."]) + &sig("Omar"),
        ago_min: 60 * 150, read: true, other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(0),
        subject: "Customer interview notes".into(),
        html: para(&["Notes from the five customer interviews are attached. Top theme: faster search. @Alex Morgan I'd love your take on theme 2."]) + &sig("Priya"),
        ago_min: 60 * 24 * 8, read: true,
        attachments: vec![("Interview notes.txt", "text/plain", 0, Some(NOTES_TXT))],
        flag: FlagStatus::Flagged, due_in_days: Some(-1),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(1),
        subject: "Architecture review: notification service".into(),
        html: para(&["Design doc attached. Main question: push via WebSocket or long-poll fallback?"]) + &sig("Marcus"),
        ago_min: 60 * 24 * 10, read: true,
        attachments: vec![("Notification service design.docx", DOCX, 180_000, None)],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Lumen Studio", "hello@lumen.example"),
        subject: "Invoice INV-2048".into(),
        html: para(&["Invoice INV-2048 for $12,400.00 is attached. Due in 30 days."]),
        ago_min: 60 * 24 * 14, read: true, other: true,
        attachments: vec![("INV-2048.pdf", "application/pdf", 88_000, None)],
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(4),
        subject: "Launch event venue shortlist".into(),
        html: para(&["Shortlisted three venues; the warehouse space downtown is my favourite."]) + &sig("Hannah"),
        ago_min: 60 * 24 * 20, read: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(5),
        subject: "FY27 planning kickoff".into(),
        html: para(&["Kickoff deck attached. First submissions due November 15."]) + &sig("Wei"),
        ago_min: 60 * 24 * 35, read: true, flag: FlagStatus::Flagged, due_in_days: Some(7),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(10),
        subject: "Badge photos this Friday".into(),
        html: para(&["Photographer will be in the lobby 10–12 on Friday."]) + &sig("Grace"),
        ago_min: 60 * 24 * 45, read: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(2),
        subject: "Moodboard for the spring campaign".into(),
        html: para(&["A first pass at the moodboard. Feedback welcome before Monday."]) + &sig("Elena"),
        ago_min: 60 * 24 * 70, read: true,
        ..Default::default()
    })?;
    // Snoozed until tomorrow morning: shows the Snoozed folder.
    s.add(inbox, M {
        from: person(9),
        subject: "Reference check for the design candidate".into(),
        html: para(&["Hi Alex, could you do a 20-minute reference call sometime this week?"]) + &sig("Liam"),
        ago_min: 60 * 6, read: true, snooze_hours: Some(20),
        ..Default::default()
    })?;

    // ---------------------------------------------------------------- custom folders
    s.add(launch, M {
        from: person(0),
        subject: "Launch checklist v2".into(),
        html: para(&["Updated checklist with owners for each item."]) + &sig("Priya"),
        ago_min: 60 * 24 * 3, read: false,
        ..Default::default()
    })?;
    s.add(launch, M {
        from: person(4),
        subject: "Press release draft".into(),
        html: para(&["Draft press release attached for comments."]) + &sig("Hannah"),
        ago_min: 60 * 24 * 6, read: true,
        attachments: vec![("Press release draft.docx", DOCX, 54_000, None)],
        ..Default::default()
    })?;
    s.add(projects, M {
        from: person(3),
        subject: "Spike results: local full-text search".into(),
        html: para(&["The FTS5 spike looks promising: prefix queries on 200k messages come back in under 20 ms."]) + &sig("Tomás"),
        ago_min: 60 * 24 * 12, read: true,
        ..Default::default()
    })?;
    for (i, (what, total)) in [("Seattle flight", "$412.60"), ("Hotel deposit", "$180.00"), ("Team lunch", "$96.40")].iter().enumerate() {
        s.add(receipts, M {
            from: EmailAddress::new("Skyline Air", "no-reply@skyline.example"),
            subject: format!("Receipt: {what}"),
            html: para(&[&format!("Total charged: {total}"), "Thank you for your purchase."]),
            ago_min: 60 * 24 * (4 + i as i64 * 9), read: true,
            ..Default::default()
        })?;
    }

    // ---------------------------------------------------------------- Sent Items
    let marcus = person(1);
    s.add(sent, M {
        from: me.clone(),
        to: vec![marcus.clone()],
        subject: "RE: Code freeze moved to Friday".into(),
        html: para(&["Hi Marcus,", "Release notes are mine — I'll have a draft by Thursday morning."]) + &sig("Alex"),
        ago_min: 40, read: true,
        ..Default::default()
    })?;
    // Replies come after the messages they answer; a forward shows the
    // forwarded icon on the original.
    for (to, subject, body, ago_min) in [
        (person(5), "RE: Budget variance — September actuals", "Thanks Wei, tomorrow at 10 works. Can you include the Q4 forecast too?", 240),
        (person(3), "RE: PR #2291: offline sync for drafts", "Left a few comments on the retry backoff. Otherwise looks great.", 60 * 118),
        (person(6), "Kickoff date options", "Could we do the kickoff on the 3rd or the 5th? Both work on our side.", 60 * 71),
        (person(8), "RE: NDA for Lumen partnership", "Section 4 redlines are fine with me. Please go ahead.", 60 * 28),
        (person(11), "RE: Hotel options near the Seattle office", "The second one looks perfect, thanks Omar.", 60 * 148),
        (person(1), "FW: Customer interview notes", "Marcus, FYI: theme 2 overlaps with the search work your team scoped.", 60 * 24 * 8 - 90),
    ] {
        s.add(sent, M {
            from: me.clone(),
            to: vec![to],
            subject: subject.into(),
            html: para(&[body]) + &sig("Alex"),
            ago_min,
            ..Default::default()
        })?;
    }

    // ---------------------------------------------------------------- Archive, Deleted Items, Junk
    for (i, (from, subject)) in [
        (person(0), "Q3 retro notes"),
        (person(5), "Q3 close: final numbers"),
        (person(2), "Brand refresh kickoff recap"),
        (person(7), "Load test results for the API gateway"),
        (person(10), "Holiday schedule"),
        (person(6), "Lumen Studio onboarding pack"),
    ]
    .into_iter()
    .enumerate()
    {
        s.add(archive, M {
            from,
            subject: subject.into(),
            html: para(&["Filing this for reference.", BODY_LINES[i % BODY_LINES.len()]]),
            ago_min: 60 * 24 * (5 + i as i64 * 6),
            ..Default::default()
        })?;
    }
    for (i, (from, subject)) in [
        (EmailAddress::new("Northwind Weekly", "newsletter@northwind.example"), "Last week at Northwind"),
        (person(11), "Calendar hold: travel planning"),
        (EmailAddress::new("Skyline Air", "deals@skyline.example"), "Fall fares from $89"),
        (person(3), "Old build failures"),
    ]
    .into_iter()
    .enumerate()
    {
        s.add(deleted, M {
            from,
            subject: subject.into(),
            html: para(&["This message was deleted."]),
            ago_min: 60 * 24 * (1 + i as i64 * 3),
            ..Default::default()
        })?;
    }
    for (i, (from, subject)) in [
        (EmailAddress::new("Prize Center", "winner@prize-center.example"), "You've been selected!!! Claim your reward"),
        (EmailAddress::new("Account Security", "security@acc0unt-verify.example"), "Unusual sign-in activity: verify now"),
        (EmailAddress::new("Crypto Daily", "news@crypto-daily.example"), "This coin is up 4,000%"),
    ]
    .into_iter()
    .enumerate()
    {
        s.add(junk, M {
            from,
            subject: subject.into(),
            html: para(&["Click the link below to continue.", "<a href=\"https://phish.example/login\">https://phish.example/login</a>"]),
            ago_min: 60 * (8 + i as i64 * 30),
            read: false,
            ..Default::default()
        })?;
    }

    // ---------------------------------------------------------------- older history
    for i in 0..320usize {
        let topic = TOPICS[(i * 7) % TOPICS.len()];
        let subject = SUBJECTS[(i * 5) % SUBJECTS.len()].replace("{}", topic);
        let newsletter = i % 9 == 4;
        let (from, other) = if newsletter {
            let (n, a) = NEWSLETTERS[i % NEWSLETTERS.len()];
            (EmailAddress::new(n, a), true)
        } else {
            (person((i * 5 + 3) % 12), false)
        };
        let subject = if newsletter { format!("{}: {topic}", from.name) } else { subject };
        let folder = match i % 10 {
            7 | 8 => archive,
            9 if i % 20 == 9 => projects,
            _ => inbox,
        };
        let html = para(&[
            &format!("Hi Alex, about {topic}:"),
            BODY_LINES[i % BODY_LINES.len()],
            BODY_LINES[(i + 3) % BODY_LINES.len()],
        ]) + &sig(from.name.split(' ').next().unwrap_or(""));
        s.add(folder, M {
            from,
            subject,
            html,
            // From 8 days back to about 14 months back, roughly 1.3 days apart.
            ago_min: 60 * 24 * 8 + (i as i64) * 60 * 31 + ((i * 37) % 600) as i64,
            read: i % 23 != 0,
            other,
            flag: if i % 41 == 0 { FlagStatus::Flagged } else { FlagStatus::NotFlagged },
            categories: match i % 13 {
                0 => vec!["Blue category"],
                5 => vec!["Green category"],
                8 => vec!["Yellow category"],
                _ => Vec::new(),
            },
            attachments: if i % 11 == 0 { vec![("Notes.docx", DOCX, 40_000 + (i as i64) * 300, None)] } else { Vec::new() },
            // Each generated message is its own conversation.
            conversation: Some(format!("demo-history-{i}")),
            ..Default::default()
        })?;
    }

    // ---------------------------------------------------------------- drafts
    let mut draft = Draft {
        account_id: acct,
        to: vec![person(6)],
        subject: "Kickoff agenda".into(),
        body_html: "<p>Hi Sofia,</p><p>Proposed agenda for the kickoff:</p><ul><li>Goals</li><li>Timeline</li></ul>".into(),
        ..Default::default()
    };
    store.save_draft(&mut draft)?;
    let mut draft2 = Draft {
        account_id: acct,
        to: vec![person(0), person(4)],
        subject: "Launch comms timeline".into(),
        body_html: "<p>Hi both,</p><p>Here's a first cut at the comms timeline for launch week.</p>".into(),
        ..Default::default()
    };
    store.save_draft(&mut draft2)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn demo_mail_covers_the_interesting_cases() {
        let store = Store::open_in_memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        super::super::seed_demo(&store, now).unwrap();
        let acct = store.accounts().unwrap()[0].id;
        let inbox = store.folder_by_role(acct, FolderRole::Inbox).unwrap().unwrap();
        assert!(inbox.total_count > 200, "inbox has {}", inbox.total_count);
        let all = store.messages(&MessageQuery::default()).unwrap();
        // A three-message conversation.
        assert!(store.conversation("demo-Venue contract — deposit due Friday").unwrap().len() >= 3);
        // The meeting invitation points at the seeded event.
        let invite = all.iter().find(|m| m.meeting_kind == Some(MeetingMessageKind::Request)).expect("invitation");
        let event = store.event(invite.meeting_event_id.unwrap()).unwrap().unwrap();
        assert!(event.title.starts_with("Interview: Senior Engineer"));
        // Newsletters with remote images sit in Other.
        let newsletter = all.iter().find(|m| m.subject.starts_with("This week at Northwind")).unwrap();
        assert_eq!(newsletter.inference, InferenceClass::Other);
        let html = store.message_body(newsletter.id).unwrap().unwrap().html.unwrap();
        assert!(crate::html::sanitize_email_html(&html, false).blocked_remote_content);
        assert!(all.iter().any(|m| m.mentions_me));
        assert!(all.iter().any(|m| m.importance == Importance::Low));
        assert!(all.iter().any(|m| m.to.len() + m.cc.len() >= 10));
        assert!(all.iter().any(|m| m.flag == FlagStatus::Flagged && m.flag_due.is_some()));
        assert!(all.iter().any(|m| m.flag == FlagStatus::Complete));
        // Every well-known folder has something, and the Snoozed folder exists.
        for role in [FolderRole::Sent, FolderRole::Archive, FolderRole::Deleted, FolderRole::Junk, FolderRole::Drafts] {
            assert!(store.folder_by_role(acct, role).unwrap().unwrap().total_count > 0, "{role:?} is empty");
        }
        assert!(store.snoozed_folder(acct).unwrap().is_some());
        // Some attachments can be downloaded.
        let csv = all.iter().find(|m| m.subject.starts_with("Budget variance")).unwrap();
        let body = store.message_body(csv.id).unwrap().unwrap();
        assert!(body.attachments.iter().any(|a| store.attachment_data(a.id).unwrap().is_some()));
        let favs: Vec<String> = store.favorites().unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(favs, vec!["Inbox", "Sent Items", "Deleted Items", "Q4 Launch"]);
        // Replies are threaded: the first venue message was answered, the
        // interview notes were forwarded.
        let venue = store.conversation("demo-Venue contract — deposit due Friday").unwrap();
        let responses = store.last_responses(&[venue[0].id]).unwrap();
        assert!(responses.get(&venue[0].id).is_some_and(|r| !r.forwarded));
        let notes = all.iter().find(|m| m.subject == "Customer interview notes").unwrap();
        assert!(store.last_responses(&[notes.id]).unwrap()[&notes.id].forwarded);
    }
}
