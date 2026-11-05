# Packages and Topics: orders and topics found on this computer

Two views in Mail, both worked out on this computer from mail that is already
downloaded:

1. **Packages** lists orders and deliveries found in order confirmations and
   shipping emails, grouped by order: where each one is, when it should
   arrive, its tracking numbers and the emails it was found in.
2. **Topics** sorts mail by what it's about (People, Orders & shipping,
   Receipts & bills, Finance, Travel, Events & invitations, Social,
   Notifications & alerts, Newsletters & promotions, Other) across folders.

Neither ever moves mail, changes it or tells a server anything. Both are only
views over mail you have: they can be switched off (Settings › Mail ›
Packages / Topics), which deletes what was worked out and keeps your
corrections. Nothing in either feature uses the network. The one page they
ever open is a carrier's own tracking page, from a fixed template, when you
click Track.

One other feature uses topics to move mail, and only when you ask it to:
auto-archive's "Sort into topic folders" (Settings › Mail › Auto-archive)
files the old mail it archives into a folder for its topic inside Archive
(`Store::auto_archive`, `Store::sort_topics`). It works out topics for that
mail even while Topics is off. Mail that fits no topic (Other) stays in Archive
itself.

Code: `crates/axentra-core/src/orders/` and `topics/` (pure logic: no I/O, no
store), `crates/axentra-core/src/scan.rs` (text preparation shared with junk),
`crates/axentra-core/src/store/orders.rs` and `store/topics.rs` (storage and
the background workers), `crates/axentra-ui/src/mail/insights_pass.rs` (when
the workers run), `mail/packages/` (the Packages page), the Views section in
`mail/folders.rs`, topic lists in `mail/model.rs` and `mail/list/header.rs`,
banners and the topic pill in `mail/reading/mod.rs`, and the settings pages
in `settings/mail.rs` (`PackagesPage`, `TopicsPage`).

## Data flow

```
new or changed mail (local, Graph or IMAP)
  └─ triggers drop what was worked out from what changed (see Triggers)
background pass (UI, off the UI thread; mail/insights_pass.rs, scheduled like
the junk pass: batches until nothing is due, idle until mail changes, a
failed batch retried after 30 s)
  ├─ Store::extract_orders_pending (Packages on)   250 messages, newest first
  │     orders::extract_guarded → order_extractions (+ order_keys, orders_dirty)
  ├─ Store::rebuild_dirty_orders  (Packages on)    200 changed keys
  │     orders::group + orders::build_order → orders (+ order_messages,
  │     order_group_keys)
  └─ Store::classify_topics_pending (Topics on)    250 messages, newest first
        topics::classify_guarded → topic_assignments
Packages page / folder pane: Store::orders, orders_summary, orders::section
topic lists: Store::messages with MessageQuery.topic; counts: Store::topic_counts
reading pane: Store::order_for_message, extraction_for, topic_for
```

Everything reads the cached junk check (`junk_assessments`); neither worker
runs the check itself. **Nothing is shown before the junk check has looked at
a message:** an order email joins an order only once it has been checked, and
a message is sorted into a topic only once it has been checked (the reading
pane checks the message before it sorts it). A later check that finds signs
of phishing re-extracts the message's order email and re-sorts it, through
the pending queries below. A message you reported as phishing counts as one
with signs of phishing (also after you move it out of Junk Email without
"Not junk"). A check that is thrown away to be redone ("Forget learned
data", a changed header) changes nothing until it is redone: the order and
topic stay as they were.

**Pending work lives only in the store.** A message is pending when it has no
row with the current version, or (for order emails, for topics a phishing
verdict changes, and for any topic when the message became phishing or
stopped being it) when its junk check changed since. Writing a batch records
exactly what the next query compares, so a second run after draining does
nothing. Leaving Mail stops the pass; coming back resumes it where the store
says. A batch worked out while a feature was switched off, a sender was
ignored or a topic corrected isn't written; the next batch redoes it.

**Failures.** Extraction and classification are wrapped in `catch_unwind`: a
message that makes them fail is recorded as incomplete (an extraction that is
no order) or as Other, and the pass goes on. Logs carry the error text only,
never message content.

## Tables

All in the profile's database, next to the mail.

| Table | Holds | Backed up |
| --- | --- | --- |
| `order_extractions` | Per message: version, the junk version it was gated on, kind (`none`, `order`, `suppressed`), whether it may only join an order, confidence, status, the `orders::Extraction` as JSON | No (derived) |
| `order_keys` | Per message: its order number and tracking number keys | No |
| `orders`, `order_messages`, `order_group_keys` | Built orders (`orders::Order` as JSON), their messages, and every key of each | No |
| `orders_dirty` | Keys whose orders need rebuilding | No |
| `order_user_state` | Your changes per order key: removed from Packages, marked delivered or not | Yes |
| `order_sender_ignores` | "Don't track orders from…" addresses and domains | Yes |
| `topic_assignments` | Per message: version, junk version, topic, confidence, source, reasons, and `stale` (shown, but to be sorted again) | No |
| `topic_sender_rules` | Topic corrections per address or domain (per account, or all accounts) | Yes |
| `topic_message_overrides` | Topic corrections for one message | No (message ids change on restore) |
| `topic_dirty_senders` | Addresses whose contact changed (their mail is sorted again) | No |
| `orders_regate_senders` | Junk sender rules changed since the last extraction batch (their senders' order emails are read again) | No |

The index `messages_received_at` serves the newest-first pending queries.

Order keys are `"{account}:o:{merchant key}:{ORDER NUMBER}"` and
`"{account}:t:{carrier id}:{NUMBER}"`. An order is addressed by its canonical
key (the smallest `o:` key, else the smallest `t:` key), never by its row id,
which changes when it is rebuilt; your changes are matched against every key
of a rebuilt group.

## Versions and levels

- `orders::EXTRACT_VERSION` (stored as `o6`): bump it when what is extracted
  changes; every message is extracted again.
- `topics::CLASSIFY_VERSION` and the kv `topics.context_rev` (stored as
  `t4-c{rev}`): bump the version when the rules or lexicon change.
  "Sort my mail again" and "Forget all corrections" bump the revision.
- kv `orders.schema` (`2`) and `topics.schema` (`3`): the trigger level. A
  later change to a trigger's body needs a `DROP TRIGGER` gated on a higher
  level, since triggers are created with `IF NOT EXISTS`.

## Triggers

Orders (dropping an extraction drops its keys, which marks them dirty):

| Trigger | When | Does |
| --- | --- | --- |
| `orders_reextract_message` | the sender, subject or account of a message changes | drop its extraction |
| `orders_reextract_body_insert` / `_update` / `_delete` | its body arrives, changes (not an unchanged download) or goes | drop its extraction |
| `orders_regroup_on_move` | it moves to another folder | mark its keys dirty |
| `orders_regroup_on_folder_move` | a folder moves (deleted into Deleted Items, or back out) | mark the keys of the mail in it and every folder under it dirty |
| `orders_regroup_on_key_delete` | a key goes (also on delete, through the foreign key) | mark the key dirty |
| `orders_regate_rule_insert` / `_update` / `_delete` | a junk sender rule comes, changes or goes | queue its pattern; the next extraction batch drops the order (and suppressed) extractions of the senders all queued rules cover in one pass, so an allow rule's exception to the phishing gate is weighed again (a restore writing hundreds of rules doesn't read the order emails once per rule) |

When a rebuild makes a merchant better known (its first order, or its first
order a carrier's own mail confirms), its mail that wasn't taken
for an order email is extracted again: mail is read newest first, so a shop's
later mail can be read before the order it belongs to exists.

Topics:

| Trigger | When | Does |
| --- | --- | --- |
| `topics_reclassify_message` | sender, subject, account, meeting, To or Cc change | drop its topic |
| `topics_reclassify_body_*`, `topics_reclassify_attachment_*` | body or attachments change | drop its topic |
| `topics_follow_order_insert` / `_update` / `_delete` | order detection changes its mind about the message | drop its topic; when a "not an order" extraction goes (order tracking switched off), mark its topic `stale` instead, so it stays shown until sorted again |
| `topics_follow_order_none` | order detection finds no order in mail sorted into Orders by its words alone (before it was looked at) | drop that topic |
| `topics_people_contact_*` | a contact is added, changed or removed | queue that contact's addresses |

Read state, flags, categories, pins and moves never redo an extraction or a
topic. There is no trigger on `junk_assessments`: the pending queries compare
junk versions instead, and only for order emails and for topics a phishing
verdict changes (Orders, Finance, Receipts, and mail already sorted as
suspicious). Extractions are written with an upsert, never `INSERT OR
REPLACE`, so the topic triggers see an order email turn into none.

## Packages (`orders`)

### Finding an order email (`orders::extract`)

The message is prepared as text (`scan::prepare`): HTML is never rendered,
links are read but never followed, and inputs are capped (512 KB of HTML,
96 KB of text, 2 KB of subject, 200 links, 200 tracking candidates, 10
tracking numbers, 10 items). Cuts land on character boundaries and every
scanner is linear. The "too large to download" placeholder is read as its
subject only.

Gates first. Each makes the message **suppressed**: never an order, and never
part of one.

1. The sender is on "Don't track orders from…".
2. The junk check found signs of phishing (unless you allowed the sender; then
   it may still only join an order), or you reported it as phishing (whatever
   the allow rules say).
3. The sender's name or the start of the subject claims a carrier (UPS, FedEx,
   DHL, Royal Mail, …) on what reads as a parcel notice (a parcel, delivery,
   tracking or fee word, or a tracking number), but the mail doesn't come
   from that carrier's domains. A carrier named in the body ("shipped with
   UPS") is never a claim, nor is a word that only looks like one
   ("follow-ups", the German "Ups"): the short acronyms count in capitals
   only. A shop's own email that names its carrier in the subject ("Shipped
   via UPS: order #BL-48213") is not a claim either.
4. A **delivery-fee lure**: a parcel held back (on hold, customs, redelivery,
   a missed delivery) and a demand for money (pay, fee, duty) within two
   sentences of it anywhere in the body (padding, hidden or not, can't push
   it out of sight), and a link, from a sender that is neither a
   carrier nor a shop on its own domain with an order a carrier confirmed:
   mail from the carrier's own domain about the same parcel is part of the
   order. A carrier's From address counts only when the junk check's
   trusted `Authentication-Results` vouch for it (DMARC pass, or a DKIM pass
   signed by that domain; `Auth::vouches_for`): anyone can write
   "UPS <alerts@ups.com>". Accounts whose provider writes no such header
   (Microsoft, local) therefore never have carrier mail exempted. A
   carrier's mail read before its junk check is read again after it. A
   carrier's domain is never a trusted shop: each of its emails vouches for
   itself. The shop's own tracking numbers don't count, check digit or not
   (anyone can work one out for a number they made up). A free-mail
   domain is never such a shop: a free-mail seller's orders are keyed by its
   address, and vouch for nobody else at that provider. An
   order number in the text doesn't change that, and a price alone ("Free
   delivery over £40", a receipt's delivery fee) is no demand. The reading
   pane says "This looks like a fake delivery notice".
5. A **fake order**: what would be an order email, from a sender the junk
   check found posing as someone else (a shop's name on another domain, a
   look-alike domain, replies that go elsewhere), even when it only says
   Caution, unless you allowed the sender. Its order and tracking numbers
   are free to make up. Such mail never gets the Orders, Finance or Receipts
   topic either.

Then the order number (after a label such as "Order #", "Order number",
"Bestellnummer", "注文番号"; at least 3 digits; not a date, price, phone
number or tracking number), tracking numbers, the status, the expected
delivery and the items. Confidence adds up:

| Sign | Weight |
| --- | --- |
| Labelled order number | +0.45 |
| Tracking number with a valid check digit (from a carrier link: +0.40; labelled only: +0.30) | +0.50 |
| Status phrase in the subject (in the first 4 KB of the body: +0.10) | +0.25 |
| A shop you already have orders from | +0.15 |
| Order or shipping sender (`orders@`, `shipping@`, `no-reply@`, …) | +0.10 |
| A carrier's own domain, vouched for (see 4. above) | +0.20 |
| A reply (Re:, AW:, …) or a contact's answer | −0.35 |
| Newsletter shape (unsubscribe link, many links or a sale word, no tracking; not with an order number and a status in the subject or an order sender) | −0.30 |
| A junk caution | −0.25 |
| Sent from one of your own addresses | −0.20 |

A number right after an order label stays the order number unless its
shape alone names a carrier (UPS, USPS, S10): the DHL and FedEx check digits
hold for one in 7 or 10 numbers, so a shop's numeric order number near
"track your" or "DHL" would otherwise pass for a tracking number.
"Confirmation number" is also a hotel's and an airline's label: it counts
only in mail about an order (an order word, or a tracking number) and never
in a booking (reservation, flight, check-in, hotel, …).

It is an order email at 0.70 or more with an order number or a tracking
number. A tracking number alone without a check digit, or under a junk
caution, can join an order but never start one.

**Tracking numbers.** UPS (`1Z`, check digit), USPS IMpb (check digit; a
`420`+ZIP routing prefix is stripped), UPU S10 (`AB123456785US`, check digit;
the country code picks Royal Mail, Deutsche Post, La Poste, PostNL, Canada
Post, Japan Post, China Post or another postal service), FedEx 12 / 15 digits
and SSCC (check digit and FedEx context), DHL Express 10 digits (check digit
and DHL context), and labelled formats without a check digit (DHL eCommerce,
DHL Paket, Amazon Logistics, Canada Post, PostNL, Evri, DPD, OnTrac). Not a
tracking number: a phone number, the order number, part of an email address,
a quoted reply line, or a token repeated across tracking pixels of three
other sites.

**Status** comes from the subject first, else the first 4 KB of the body,
never a footer. In priority order: Refunded, Cancelled, Returned, Delivery
problem, Delayed, Delivered, Out for delivery, Ready for pickup, In transit,
Shipped, Ordered, with phrases in English, German, French, Spanish, Japanese
and Chinese. "Not delivered" is a delivery problem; "when it has shipped" or
"will be delivered" is a promise, not a status.

**Expected delivery** follows a label ("Arriving", "Estimated delivery",
"お届け予定日", …): today, tomorrow, a weekday, month names in English, German,
French and Spanish, ISO dates, `2026年10月22日`, ranges ("Nov 2 – 5",
"between … and …"). It is read from the subject and the first 4 KB of the
body, never a footer, and not after "Order by …" in the same sentence (a
promotion's deadline). A numeric date is used only when it can't be misread
(day first only for `.co.uk`, `.de`, `.fr`, `.es`, `.it` and `.nl` senders,
with a year); a dotted date is always day first and needs a year or a closing
dot ("22.10."). A time of day ("9:00 AM", "9.30am") is never a date, nor the
end of a range. A date without a year must land between a week before and
120 days after the email, and a weekday that doesn't match the date drops it.

### Orders (`orders::group`, `build_order`, `section`)

Emails are grouped by shared keys (order number, tracking number). Two
different order numbers of the same shop are never merged, even when they
share a consolidated shipment. Copies of one email in two folders count once.
Only order emails the junk check has seen, outside Junk Email, Deleted Items
and your own mail (and any folder under them, such as a folder deleted into
Deleted Items), are grouped: moving an email to Junk or Deleted takes it
out of its order, moving it back restores it, and deleting it prunes the
order. A group keeps its newest 200 emails.

The status is the latest email's, except that nothing but a delivery problem,
return or refund overrides Delivered (a late "Shipped" reminder doesn't), a
later Shipped overrides Cancelled (a partial cancellation), a later "Thank
you for your order" (a review request, a receipt sent again) never reopens a
cancelled, returned or refunded order, and only Out for delivery or Delivered
clear Delayed or a delivery problem. With several
shipments, the order is Delivered only when all are; otherwise it is
partly delivered. Your "Mark as delivered / not delivered" wins, and
"Remove from Packages" hides the order also when new mail about it comes in.

Sections: **Needs attention** (delayed, a delivery problem, or two days past
its expected date), **Arriving soon** (out for delivery, ready for pickup,
expected within two days, or delivered today), **On the way**, **Ordered**,
**Delivered** (also an order with no news for 45 days: "No recent updates"),
**Cancelled or refunded**. Delivered, returned, cancelled and refunded
orders leave after Settings › Mail › Packages › "Keep delivered packages for"
(a week, 30 days, 90 days or a year). Orders and mail older than 400 days aren't looked at.

**Merchants.** The shop is the sender's registrable domain, shown under the
shop's name so a mismatch is visible. Mail relayed by a known marketplace is
filed under the shop's name instead; mail from any other `shops.` or `mailer.`
host is filed under the shop's name within that host's domain, so a shop's
name written on someone else's domain is never the same shop. A carrier's own mail is filed under the
carrier, named after the shop it mentions when it does ("Package from
Brightleaf Home").

## Topics (`topics`)

The first rule that decides wins:

1. Your correction for this message.
2. Your correction for the sender: the exact address (this account's before
   all accounts'), then the most specific domain. A domain covers its
   subdomains on a label boundary (`example.com` covers `mail.example.com`,
   not `evilexample.com`). Free mail domains (gmail.com, outlook.com, …) can't
   have a domain correction. Your choice wins even over warning signs; the
   reasons still say so.
3. Signs of phishing, or mail Packages suppressed as a lure or carrier
   impersonation: **Other**. Phishing never lands in Orders, Finance or
   Receipts. Mail Packages hasn't looked at (order tracking off, older than
   400 days, or a sender you chose not to track) goes through the same
   lure and carrier-impersonation checks here; otherwise a sender not
   tracked isn't suspicious: its mail is sorted as if order tracking were
   off for it.
4. A meeting invitation or response: Events & invitations.
5. An order email: Orders & shipping, at its confidence.
6. Someone you know (a contact, or someone you wrote to) writing personally:
   not from an automated address (`noreply`, `notifications`, `info`, …), no
   unsubscribe link, at most 20 recipients. People.
7. Scored signals (`topics/lexicon.rs`): subject words +0.35 each (at most
   0.7), phrases in the first 4 KB of the body +0.15 each (at most 0.45),
   the sender's local part +0.2, domain hints +0.2, and structure signs (an
   amount after "total" or "invoice", a booking code, a calendar attachment,
   a verification code, an unsubscribe link). The best topic needs 0.55 and a
   lead of 0.15; otherwise the message is Other. An order, receipt, bank,
   travel or alert email with an unsubscribe footer stays what it is rather
   than a newsletter; on a tie Receipts beats Finance, and Finance beats
   Notifications.
8. Otherwise Other.

A phrase inside another topic's longer phrase counts for that topic only
("event reminder" is Events, not also a Notifications "reminder"; "save
your seat" is not Travel's "seat"). An Events phrase in the subject that the
body repeats adds 0.1.

With order tracking off, the Orders words stand in for order detection.
"Why?" next to the topic pill lists the reasons.

**Corrections.** Change topic (message menu, the topic pill, or the keyboard
dialog) applies to the message, the sender (the default) or the sender's
domain. The scope is chosen in the dialog and remembered; the menus say
what a pick applies to. A remembered domain scope applies only where the
messages share one domain that isn't free mail; otherwise the sender's
address is corrected. A sender or domain correction updates the mail already sorted in
place, so nothing is worked out again and nothing flickers, and new mail from
that sender gets it too. Removing one sorts its mail again. Undo puts the
previous correction back. The dialog may suggest a domain correction after
two address corrections in the same domain; it never makes one by itself.

**Not re-sorted on their own:** changes to the addresses you've written to
(`recent_addresses`) don't re-sort mail; a contact change re-sorts only that
contact's mail. "Sort my mail again" re-sorts everything.

## Privacy guarantees

- `orders/`, `topics/` and `scan.rs` do no I/O: a test reads their sources and
  fails on `std::net`, `std::fs`, `std::process`, HTTP crates or the store.
- Links in mail are never stored, shown as Track targets or followed. A
  tracking number read from a carrier link is kept; the link isn't (a test
  checks the stored extraction contains no `http`).
- The only URL ever opened is `Carrier::tracking_url`, built from a fixed
  template after the number is validated again, and only on a click (or the T
  key on a card). Carriers without a tracking page (Amazon Logistics, Evri,
  DPD, China Post, other postal services) offer "Track from the order email".
- Nothing writes to the mail itself: `messages.json` is byte-identical after
  every feature operation (test `messages_json_untouched`), and nothing changes
  categories, Focused / Other or folders.
- Backups carry your corrections and changes (`topic_sender_rules`,
  `order_user_state`, `order_sender_ignores`) but nothing worked out from the
  mail (test `topic_corrections_and_order_changes_round_trip_without_derived_data`).

## Switching off

Settings › Mail › Packages › "Find orders and deliveries in my mail" and
Settings › Mail › Topics › "Sort mail by topic". Off stops the worker, hides
the views (folder pane, the pinned desktop view's switcher, the View ribbon
buttons), the order and fake-delivery banners, the topic pill and the Change
topic menu, and deletes what was worked out (`Store::clear_orders`,
`Store::clear_topics`). Restoring a backup's settings with a feature off
deletes the same. Your corrections, order changes and ignored senders stay.
On again rebuilds everything from the mail. "Look through my mail again"
(with tracking on) marks mail that wasn't an order email for another look
instead of deleting its extraction, so its topic stays put (a deleted
extraction takes the message's topic with it).

## Adding a carrier

1. Add the variant to `orders::Carrier` and its `id()` (stable: it is part of
   order keys), `name()`, `host()` and `tracking_url()` template, if it has a
   tracking page (`orders/carriers.rs`).
2. Add its format to `validate()` and to `resolve()` at the right place:
   formats are tried in order and the first that accepts a number wins, so
   check-digit formats go before labelled ones. Formats without a check
   digit need a label or the carrier's name nearby.
3. Add its sender domains (`sender_domains`), how mail names it (`names`, for
   impersonation claims and context) and, if it has one, its tracking link
   host and parameters (the host table in `links()`). Add it to `NAMED`.
4. Tests in `orders/carriers.rs`: known-good numbers, mutations rejected (a
   `*_with_check` helper when it has a check digit), context rules, and the
   `tracking_url` prefix.
5. Bump `orders::EXTRACT_VERSION`.

## Adding a topic signal

- Words, phrases, sender local parts and domain hints: the topic's entry in
  `topics/lexicon.rs` (`SIGNALS`). English phrases are matched
  ASCII-lowercase on word boundaries; non-English ones as written (list both
  the accented and the plain spelling).
- Structure signs: `structure()` in `topics/classify.rs`.
- Add a case to `one_message_per_topic` (and check the precedence and margin
  tests still hold), then bump `topics::CLASSIFY_VERSION`.
- A new topic also needs its `Topic` variant (`ALL`, `id`, `label`, `icon`),
  an icon in `crates/axentra-ui/icons/` (the icon test checks it), and its
  empty-list text in `mail/list/mod.rs`.

## Not done / limits

- **English-first.** Status phrases, order labels and topic words cover
  English, German, French, Spanish and Japanese (some Chinese and Korean);
  other languages fall back to tracking numbers and Other.
- **Dates** in other formats than those above are dropped rather than
  guessed.
- **Items** are read best effort from tables and "Qty:" lines.
- **No carrier lookups:** status comes only from the emails, so a parcel
  whose carrier sends no email stays at its last email's status. Track opens
  the carrier's page instead.
- **Big groups:** a group's rebuild follows at most 200 emails and 400 keys;
  a larger group (in practice only a shop that repeats one number in every
  email) is built from part of its mail. A batch of changed keys is worked
  through whole group by group: once a batch has taken 200 emails it starts
  no further group, and the keys it didn't get to wait for the next batch.
- **`recent_addresses` changes don't re-sort** mail (see Topics).
- **Domain comparison** uses the same approximate registrable-domain rule as
  junk, not the public-suffix list.
- **Icons:** `box`, `vehicle_truck`, `receipt`, `airplane` and `wallet` were
  drawn in the Fluent 20 px style for this feature; swap in the upstream
  Fluent System Icons files to match exactly.
