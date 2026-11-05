//! Demo mail: a realistic Inbox around `now` (threads, a meeting invitation,
//! newsletters in Other, flags, pins, categories, attachments, order and
//! shipping mail from fictional shops, an example for every topic), content in
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
    reply_to: Vec<EmailAddress>,
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
            reply_to: Vec::new(),
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
            reply_to: m.reply_to.clone(),
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

/// Synthetic tracking numbers with valid check digits (the tests recompute
/// them). Carriers are only ever named inside the shops' own mail.
const UPS_BRIGHTLEAF: &str = "1Z4X7Q203660123459";
const FEDEX_NORTHPEAK: &str = "612900344789";
const USPS_PIXEL_PARCEL: &str = "9200190301123456789017";
const JAPAN_POST_SAKURA: &str = "EJ517382041JP";

/// The order summary table shops send: cells, not text.
fn items_table(rows: &[(&str, u32, &str)]) -> String {
    let rows: String = rows.iter().map(|(item, qty, price)| format!("<tr><td>{item}</td><td>{qty}</td><td>{price}</td></tr>")).collect();
    format!("<h3>Order summary</h3><table><tr><th>Item</th><th>Qty</th><th>Price</th></tr>{rows}</table>")
}

/// Order mail from fictional shops for Packages (five orders, each in its
/// own thread, one of them in Japanese), a delivery-fee lure and two near
/// misses that must not become orders, and an example for each topic the
/// rest of the demo doesn't already cover.
fn orders_and_topics(s: &mut Seeder, ctx: &SeedCtx, inbox: FolderId) -> Result<()> {
    let date = |days: i64| (ctx.now + Duration::days(days)).format("%A, %B %-d").to_string();
    let order = |number: &str| Some(format!("demo-order-{number}"));

    // Brightleaf Home: confirmed, shipped with UPS, out for delivery today.
    let brightleaf = EmailAddress::new("Brightleaf Home", "orders@brightleaf.example");
    s.add(inbox, M {
        from: brightleaf.clone(),
        subject: "Order confirmed: #BL-48213".into(),
        html: para(&["Hi Alex,", "Thanks for shopping with Brightleaf Home. We're getting your order ready and will let you know when it ships."])
            + &items_table(&[("Linen throw blanket", 1, "&#36;48.00"), ("Stoneware mug set", 1, "&#36;32.00")])
            + &para(&["Order number: BL-48213"]),
        ago_min: 60 * 24 * 3, other: true,
        conversation: order("BL-48213"),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: brightleaf.clone(),
        subject: "Your Brightleaf order #BL-48213 has shipped".into(),
        html: para(&[
            "Good news, Alex: your order has left our warehouse.",
            &format!("Shipped with UPS. Tracking number: {UPS_BRIGHTLEAF}"),
            &format!("Estimated delivery: {}", date(1)),
        ]),
        ago_min: 60 * 24 * 2, other: true,
        conversation: order("BL-48213"),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: brightleaf,
        subject: "Out for delivery: your order #BL-48213".into(),
        html: para(&["Your package is out for delivery today.", &format!("UPS tracking number {UPS_BRIGHTLEAF}")]),
        ago_min: 90, read: false, other: true,
        conversation: order("BL-48213"),
        ..Default::default()
    })?;

    // Northpeak Outfitters: ordered, on its way with FedEx, delivered.
    let northpeak = EmailAddress::new("Northpeak Outfitters", "shop@northpeak.example");
    s.add(inbox, M {
        from: northpeak.clone(),
        subject: "Thanks for your order #NP-20931".into(),
        html: para(&["Hi Alex,", "We've received your order and we're packing it now."])
            + &items_table(&[("Trail running jacket", 1, "&#36;129.00"), ("Merino hiking socks, 3 pairs", 1, "&#36;36.00")]),
        ago_min: 60 * 24 * 9, other: true,
        conversation: order("NP-20931"),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: northpeak.clone(),
        subject: "Your order NP-20931 is on its way".into(),
        html: para(&["Your order is on its way.", &format!("FedEx tracking number: {FEDEX_NORTHPEAK}")]),
        ago_min: 60 * 24 * 7, other: true,
        conversation: order("NP-20931"),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: northpeak,
        subject: "Delivered: your Northpeak order #NP-20931".into(),
        html: para(&[&format!("Your package was delivered. FedEx tracking number {FEDEX_NORTHPEAK}"), "Enjoy the trails!"]),
        ago_min: 60 * 24 * 2, other: true,
        conversation: order("NP-20931"),
        ..Default::default()
    })?;

    // Pixel & Parcel Books: confirmed, then delayed (USPS) with a new date.
    let pixel_parcel = EmailAddress::new("Pixel & Parcel Books", "orders@pixelparcel.example");
    s.add(inbox, M {
        from: pixel_parcel.clone(),
        subject: "Order #PP-77310 confirmed".into(),
        html: para(&["Thank you for your order!"])
            + &items_table(&[("The Lighthouse Keeper's Atlas (hardcover)", 1, "&#36;34.00"), ("Field Notes on Small Gardens", 1, "&#36;22.50")]),
        ago_min: 60 * 24 * 6, other: true,
        conversation: order("PP-77310"),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: pixel_parcel,
        subject: "Delayed: order #PP-77310 has a new delivery date".into(),
        html: para(&[
            "Sorry, your books are running a little late.",
            &format!("USPS tracking number {USPS_PIXEL_PARCEL}"),
            &format!("New delivery date: {}", date(4)),
        ]),
        ago_min: 60 * 24, read: false, other: true,
        conversation: order("PP-77310"),
        ..Default::default()
    })?;

    // Harbor Lane Kitchen: cancelled, then refunded.
    let harbor_lane = EmailAddress::new("Harbor Lane Kitchen", "orders@harborlane.example");
    s.add(inbox, M {
        from: harbor_lane.clone(),
        subject: "Your order #HL-3304 has been cancelled".into(),
        html: para(&["We're sorry: the cast-iron grill pan you ordered is out of stock, so we've cancelled your order.", "Your refund follows in a separate email."]),
        ago_min: 60 * 24 * 4, other: true,
        conversation: order("HL-3304"),
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: harbor_lane,
        subject: "Refund issued for order #HL-3304".into(),
        html: para(&["Refund of &#36;27.50 issued to your original payment method.", "It can take a few business days to show up."]),
        ago_min: 60 * 24 * 3, other: true,
        conversation: order("HL-3304"),
        ..Default::default()
    })?;

    // A Japanese shop: shipped with Japan Post, full-width order number.
    let arrives = (ctx.now + Duration::days(3)).format("%Y年%-m月%-d日");
    s.add(inbox, M {
        from: EmailAddress::new("さくら雑貨", "shop@sakura-zakka.example"),
        subject: "【さくら雑貨】ご注文の商品を発送しました（注文番号：ＳＺ－１００２４５）".into(),
        html: para(&[
            "アレックス・モーガン 様",
            "このたびは さくら雑貨 をご利用いただき、誠にありがとうございます。ご注文の商品を本日発送いたしました。",
            "注文番号：ＳＺ－１００２４５",
            "配送業者：日本郵便",
            &format!("追跡番号：{JAPAN_POST_SAKURA}"),
            &format!("お届け予定日：{arrives}"),
            "商品：手ぬぐい（桜柄） × 2、豆皿セット × 1",
        ]),
        ago_min: 60 * 24, read: false, other: true,
        conversation: order("SZ-100245"),
        ..Default::default()
    })?;

    // A delivery-fee lure: never an order, whatever the junk check says.
    s.add(inbox, M {
        from: EmailAddress::new("Parcel Desk", "delivery@parcel-redelivery.example"),
        subject: "Your parcel is on hold – pay $1.99 redelivery fee".into(),
        html: para(&[
            "We couldn't deliver your package.",
            "Pay a small fee to reschedule: <a href=\"https://parcel-redelivery.example/pay\">Pay now</a>",
        ]),
        ago_min: 60 * 5, read: false,
        ..Default::default()
    })?;

    // Near misses: a promotion that says "order now", and a chat about an order.
    s.add(inbox, M {
        from: EmailAddress::new("Northpeak Outfitters", "news@northpeak.example"),
        subject: "30% off new arrivals – order now, ships free".into(),
        html: newsletter_html("Northpeak Outfitters", "30% off new arrivals", "img.northpeak.example"),
        ago_min: 60 * 24, other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: person(2),
        subject: "Re: I ordered the cake for Friday".into(),
        html: para(&["Hi Alex,", "All sorted: I placed order #12 with the bakery, pick up at 5.", "Could you bring the candles?"]) + &sig("Elena"),
        ago_min: 60 * 6, read: false,
        ..Default::default()
    })?;

    // Social, notifications and events.
    s.add(inbox, M {
        from: EmailAddress::new("Circlet", "notify@circlet.example"),
        subject: "Jordan commented on your photo".into(),
        html: para(&["Jordan Lee commented on your photo: \u{201c}Great shot!\u{201d}", "<a href=\"https://circlet.example/photos/8841\">See the comment</a>"]),
        ago_min: 60 * 3, other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Corvid Cloud", "no-reply@corvid.example"),
        subject: "Your verification code is 482913".into(),
        html: para(&["Use code 482913 to sign in. It expires in 10 minutes.", "If you didn't try to sign in, you can ignore this email."]),
        ago_min: 40, read: false,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Lakeside Makers", "events@lakeside-makers.example"),
        subject: "You're invited: Autumn maker night – RSVP".into(),
        html: para(&["Join us Thursday at 7pm for demos, snacks and a soldering corner.", "RSVP by Wednesday so we know how many chairs to set out."]),
        ago_min: 60 * 24 * 2, other: true,
        ..Default::default()
    })?;
    Ok(())
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

    // ---------------------------------------------------------------- warning signs
    // Mail the junk checks flag in the Inbox (they only advise; nothing moves).
    s.add(inbox, M {
        from: EmailAddress::new("PayPal Service", "service@paypa1-billing.example"),
        subject: "Your account is on hold".into(),
        html: para(&[
            "We noticed unusual activity and placed a temporary hold on your account.",
            "<a href=\"https://paypa1-billing.example/restore?id=88213\">https://www.paypal.com/signin</a>",
            "If you don't confirm within 24 hours your account will be closed.",
        ]),
        ago_min: 95,
        other: true,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Elena Fischer", "elena.fischer@lurnen.example"),
        subject: "Updated brand assets (new link)".into(),
        html: para(&["Hi Alex, the folder moved. Please sign in again to download the files:", "<a href=\"https://lurnen.example/share/login\">Open shared folder</a>"]),
        ago_min: 150,
        ..Default::default()
    })?;
    s.add(inbox, M {
        from: EmailAddress::new("Accounts Payable", "ap@supplies-direct.example"),
        subject: "Invoice 4471 overdue".into(),
        html: para(&["Please find the overdue invoice attached and arrange payment today."]),
        attachments: vec![("Invoice_4471.pdf.exe", "application/octet-stream", 182_000, None)],
        reply_to: vec![EmailAddress::new("", "payments@collect-now.example")],
        ago_min: 210,
        other: true,
        ..Default::default()
    })?;

    orders_and_topics(&mut s, ctx, inbox)?;

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
        // Each order is its own thread in the Inbox.
        for (number, count) in [("BL-48213", 3), ("NP-20931", 3), ("PP-77310", 2), ("HL-3304", 2), ("SZ-100245", 1)] {
            let thread = store.conversation(&format!("demo-order-{number}")).unwrap();
            assert_eq!(thread.len(), count, "{number}");
            assert!(thread.iter().all(|m| m.folder_id == inbox.id), "{number}");
        }
        // Dates in order mail follow `now`.
        let body = |subject: &str| {
            let m = all.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("{subject}"));
            store.message_body(m.id).unwrap().unwrap().text
        };
        assert!(body("Your Brightleaf order #BL-48213 has shipped").contains("Estimated delivery: Tuesday, October 20"));
        assert!(body("Delayed: order #PP-77310 has a new delivery date").contains("New delivery date: Friday, October 23"));
        let japanese = all.iter().find(|m| m.from.address == "shop@sakura-zakka.example").unwrap();
        assert!(japanese.subject.contains("ＳＺ－１００２４５"));
        assert!(store.message_body(japanese.id).unwrap().unwrap().text.contains("お届け予定日：2026年10月22日"));
        // Shops only: no sender anywhere in the demo poses as a carrier, and
        // every shop is fictional.
        for m in &all {
            let words: String = format!("{} {}", m.from.name, m.from.address).to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' }).collect();
            let words: Vec<&str> = words.split_whitespace().collect();
            for carrier in ["ups", "usps", "fedex", "dhl", "royalmail", "japanpost", "evri", "dpd", "ontrac", "postnl", "amazon"] {
                assert!(!words.contains(&carrier), "{} <{}> looks like {carrier}", m.from.name, m.from.address);
            }
            assert!(m.from.address.ends_with(".example"), "{}", m.from.address);
        }
    }

    // Order and topic mail the junk check must leave alone: a body that says
    // "shipped with UPS" is not brand impersonation, and the lure is caught by
    // the order check, not by a phishing warning (the Inbox keeps exactly
    // three of those).
    #[test]
    fn demo_order_mail_raises_no_junk_danger() {
        let store = Store::open_in_memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        super::super::seed_demo(&store, now).unwrap();
        while store.assess_pending(250).unwrap() > 0 {}
        let acct = store.accounts().unwrap()[0].id;
        let ours = store.account_messages(acct).unwrap().into_iter().filter(|m| DEMO_SUBJECTS.contains(&m.subject.as_str())).collect::<Vec<_>>();
        assert_eq!(ours.len(), DEMO_SUBJECTS.len());
        for m in ours {
            let a = store.assessment(m.id).unwrap().unwrap();
            assert_ne!(a.analysis.severity(), Some(crate::junk::analyze::Severity::Danger), "{}: {:?}", m.subject, a.analysis.findings);
        }
    }

    /// The subjects [`orders_and_topics`] adds, in order.
    const DEMO_SUBJECTS: &[&str] = &[
        "Order confirmed: #BL-48213",
        "Your Brightleaf order #BL-48213 has shipped",
        "Out for delivery: your order #BL-48213",
        "Thanks for your order #NP-20931",
        "Your order NP-20931 is on its way",
        "Delivered: your Northpeak order #NP-20931",
        "Order #PP-77310 confirmed",
        "Delayed: order #PP-77310 has a new delivery date",
        "Your order #HL-3304 has been cancelled",
        "Refund issued for order #HL-3304",
        "【さくら雑貨】ご注文の商品を発送しました（注文番号：ＳＺ－１００２４５）",
        "Your parcel is on hold – pay $1.99 redelivery fee",
        "30% off new arrivals – order now, ships free",
        "Re: I ordered the cake for Friday",
        "Jordan commented on your photo",
        "Your verification code is 482913",
        "You're invited: Autumn maker night – RSVP",
    ];

    // ------------------------------------------------------------ tracking numbers

    /// Every synthetic tracking number of the demo (and the spares the order
    /// tests use), with its carrier.
    const TRACKING: &[(&str, &str)] = &[
        ("ups", UPS_BRIGHTLEAF),
        ("ups", "1Z8F29E10355042072"),
        ("fedex", FEDEX_NORTHPEAK),
        ("fedex", "550147029134"),
        ("usps", USPS_PIXEL_PARCEL),
        ("s10", JAPAN_POST_SAKURA),
        ("s10", "LX204417384US"),
        ("s10", "CP319475029CA"),
        ("dhl", "3720419861"),
    ];

    fn digits(s: &str) -> Vec<u32> {
        s.chars().map(|c| c.to_digit(10).expect("a digit")).collect()
    }

    /// The number with its check digit recomputed from the rest, written out
    /// independently of the carrier code so a typo in either is caught.
    fn with_check(kind: &str, n: &str) -> String {
        match kind {
            // "1Z" + 15 chars + check; letters count (ascii - 63) % 10;
            // odd positions once, even positions twice.
            "ups" => {
                let sum: u32 = n[2..17]
                    .chars()
                    .enumerate()
                    .map(|(i, c)| {
                        let v = c.to_digit(10).unwrap_or_else(|| (c as u32 - 63) % 10);
                        if i % 2 == 0 { v } else { 2 * v }
                    })
                    .sum();
                format!("{}{}", &n[..17], (10 - sum % 10) % 10)
            }
            // Weights 1, 3, 7 from the right over the first 11 digits.
            "fedex" => {
                let sum: u32 = digits(&n[..11]).iter().rev().zip([1, 3, 7].iter().cycle()).map(|(d, w)| d * w).sum();
                format!("{}{}", &n[..11], sum % 11 % 10)
            }
            // Weights 3, 1 from the right over all but the last digit.
            "usps" => {
                let body = &n[..n.len() - 1];
                let sum: u32 = digits(body).iter().rev().zip([3, 1].iter().cycle()).map(|(d, w)| d * w).sum();
                format!("{body}{}", (10 - sum % 10) % 10)
            }
            // Weights 8, 6, 4, 2, 3, 5, 9, 7 over the serial; 10 → 0, 11 → 5.
            "s10" => {
                let sum: u32 = digits(&n[2..10]).iter().zip([8, 6, 4, 2, 3, 5, 9, 7]).map(|(d, w)| d * w).sum();
                let check = match 11 - sum % 11 {
                    10 => 0,
                    11 => 5,
                    c => c,
                };
                format!("{}{check}{}", &n[..10], &n[11..])
            }
            // The first nine digits mod 7.
            "dhl" => format!("{}{}", &n[..9], n[..9].parse::<u64>().unwrap() % 7),
            _ => unreachable!(),
        }
    }

    #[test]
    fn demo_tracking_numbers_have_valid_check_digits() {
        // The well-known samples check the recomputation itself.
        assert_eq!(with_check("ups", "1Z999AA10123456784"), "1Z999AA10123456784");
        assert_eq!(with_check("s10", "RR123456785US"), "RR123456785US");
        for (kind, n) in TRACKING {
            assert_eq!(with_check(kind, n), *n, "{kind} {n}");
            let last = n.len() - if *kind == "s10" { 3 } else { 1 };
            let d = n[last..last + 1].parse::<u32>().unwrap();
            let wrong = format!("{}{}{}", &n[..last], (d + 1) % 10, &n[last + 1..]);
            assert_ne!(with_check(kind, &wrong), wrong, "{kind} {wrong}");
        }
        assert_eq!(JAPAN_POST_SAKURA.len(), 13);
        assert!(JAPAN_POST_SAKURA.ends_with("JP"));
    }

    #[test]
    fn demo_tracking_numbers_pass_the_carrier_validators() {
        use crate::orders::carriers::{dhl10_with_check, fedex12_with_check, impb_with_check, s10_with_check, ups_with_check};
        use crate::orders::{Carrier, validate};
        for (kind, n) in TRACKING {
            // Recomputed with order detection's own helpers as well.
            let again = match *kind {
                "ups" => ups_with_check(&n[2..17]),
                "fedex" => fedex12_with_check(&n[..11]),
                "usps" => impb_with_check(&n[..n.len() - 1]),
                "s10" => s10_with_check(&n[..2], &n[2..10], &n[11..]),
                "dhl" => dhl10_with_check(&n[..9]),
                _ => unreachable!(),
            };
            assert_eq!(again, *n, "{kind} {n}");
            let carrier = match (*kind, &n[n.len() - 2..]) {
                ("ups", _) => Carrier::Ups,
                ("fedex", _) => Carrier::FedEx,
                ("usps", _) | ("s10", "US") => Carrier::Usps,
                ("s10", "JP") => Carrier::JapanPost,
                ("s10", "CA") => Carrier::CanadaPost,
                ("dhl", _) => Carrier::Dhl,
                _ => unreachable!(),
            };
            assert!(validate(&carrier, n), "{carrier:?} {n}");
        }
    }

    // ------------------------------------------------------------ packages and topics

    #[test]
    fn demo_orders_and_topics_are_found() {
        use crate::orders::{ExtractionKind, OrderSection, OrderStatus, SuppressReason, section};
        use crate::topics::Topic;

        let store = Store::open_in_memory().unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 19, 11, 30, 0).unwrap();
        super::super::seed_demo(&store, now).unwrap();
        let acct = store.accounts().unwrap()[0].id;
        let utc = now.with_timezone(&Utc);
        let mut rounds = 0;
        loop {
            let done = store.assess_pending(250).unwrap()
                + store.extract_orders_pending(250, utc).unwrap()
                + store.rebuild_dirty_orders(200).unwrap()
                + store.classify_topics_pending(250).unwrap();
            if done == 0 {
                break;
            }
            rounds += 1;
            assert!(rounds <= 50, "the passes never finish");
        }
        let all = store.account_messages(acct).unwrap();
        let id = |subject: &str| all.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("{subject}")).id;

        // Exactly the five orders, each where Packages should show it.
        let mut orders: Vec<_> = store
            .orders(false)
            .unwrap()
            .into_iter()
            .map(|o| (o.key.clone(), o.merchant.name.clone(), o.status, section(&o, utc, now.date_naive(), 30), o.events.len()))
            .collect();
        orders.sort();
        let key = |merchant: &str, number: &str| format!("{acct}:o:{merchant}:{number}");
        let mut want = vec![
            (key("brightleaf.example", "BL-48213"), "Brightleaf Home".to_string(), OrderStatus::OutForDelivery, Some(OrderSection::ArrivingSoon), 3),
            (key("northpeak.example", "NP-20931"), "Northpeak Outfitters".into(), OrderStatus::Delivered, Some(OrderSection::Delivered), 3),
            (key("pixelparcel.example", "PP-77310"), "Pixel & Parcel Books".into(), OrderStatus::Delayed, Some(OrderSection::NeedsAttention), 2),
            (key("harborlane.example", "HL-3304"), "Harbor Lane Kitchen".into(), OrderStatus::Refunded, Some(OrderSection::CancelledReturned), 2),
            (key("sakura-zakka.example", "SZ-100245"), "さくら雑貨".into(), OrderStatus::Shipped, Some(OrderSection::OnTheWay), 1),
        ];
        want.sort();
        assert_eq!(orders, want);
        let sakura = store.order_by_key(&key("sakura-zakka.example", "SZ-100245")).unwrap().unwrap();
        assert!(sakura.shipments.iter().any(|s| s.tracking.carrier == crate::orders::Carrier::JapanPost && s.tracking.number == JAPAN_POST_SAKURA));
        assert_eq!(sakura.expected.map(|w| w.from), chrono::NaiveDate::from_ymd_opt(2026, 10, 22));

        // The lure is caught; the promotion and the chat aren't orders.
        let lure = id("Your parcel is on hold – pay $1.99 redelivery fee");
        let x = store.extraction_for(lure).unwrap().expect("checked");
        assert_eq!(x.kind, ExtractionKind::Suppressed);
        assert_eq!(x.suppressed, Some(SuppressReason::DeliveryFeeLure));
        assert!(store.order_for_message(lure).unwrap().is_none());
        for subject in ["30% off new arrivals – order now, ships free", "Re: I ordered the cake for Friday"] {
            assert!(store.order_for_message(id(subject)).unwrap().is_none(), "{subject}");
        }

        // Every topic but Other has mail, and the examples land where expected.
        let counts = store.topic_counts().unwrap();
        for topic in Topic::ALL.into_iter().filter(|t| *t != Topic::Other) {
            assert!(counts.iter().any(|c| c.topic == topic && c.total > 0), "{topic:?} is empty: {counts:?}");
        }
        let ids: Vec<_> = all.iter().map(|m| m.id).collect();
        let topics = store.topics_for(&ids).unwrap();
        for (subject, topic) in [
            ("Out for delivery: your order #BL-48213", Topic::Orders),
            ("Your parcel is on hold – pay $1.99 redelivery fee", Topic::Other),
            ("30% off new arrivals – order now, ships free", Topic::Newsletters),
            ("Re: I ordered the cake for Friday", Topic::People),
            ("Jordan commented on your photo", Topic::Social),
            ("Your verification code is 482913", Topic::Notifications),
            ("You're invited: Autumn maker night – RSVP", Topic::Events),
            ("Your trip to Seattle is confirmed", Topic::Travel),
            ("Your monthly statement is ready", Topic::Finance),
            ("Receipt: Seattle flight", Topic::Receipts),
            ("Invoice INV-2048", Topic::Receipts),
        ] {
            assert_eq!(topics.get(&id(subject)).map(|t| t.0), Some(topic), "{subject}");
        }

        // The rest of the demo is unchanged.
        let inbox = store.folder_by_role(acct, FolderRole::Inbox).unwrap().unwrap();
        assert!(inbox.total_count > 200, "inbox has {}", inbox.total_count);
        let favs: Vec<String> = store.favorites().unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(favs, vec!["Inbox", "Sent Items", "Deleted Items", "Q4 Launch"]);
    }
}
