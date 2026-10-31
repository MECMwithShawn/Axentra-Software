# Junk email: rules, warning signs and local learning

Axentra's junk handling works on this computer only. It has three parts,
built in this order (stages A, B, C):

1. **Your rules and reports**: block / never-block rules and the labels you give
   with Report junk, Report phishing and Not junk.
2. **Warning signs**: checks of each message that never open anything.
3. **A model learned from your reports**: it suggests "looks like junk", and is
   used only after it passes fixed checks.

Only your own actions move mail. A block rule moves matching new mail to Junk
Email; your Report and Not junk clicks move what you clicked. Warning signs and
the model only **advise**: a banner in the reading pane and a marker in the
message list.

Two other choices you made earlier also act on synced mail as it arrives, and
are pushed to the server like any local change: an ignored conversation sends
new messages in it to Deleted Items, and "Always move to Other" sets Focused /
Other (on Outlook.com a Graph `inferenceClassification` change, not a folder
move). Nothing learned does.

Code: `crates/axentra-core/src/junk/` (pure logic), `crates/axentra-core/src/store/junk.rs`
(storage), `crates/axentra-ui/src/settings/mail.rs` (`JunkPage`), reading-pane
banners in `crates/axentra-ui/src/mail/reading/mod.rs`.

## Data flow

```
new mail (local, Graph or IMAP)
  └─ Store::route_incoming_in ── block rule? ── yes → Junk Email here; the sync
        │                                         shadow keeps the server folder,
        │                                         so the next push moves it on
        │                                         the server too (junk_routing
        │                                         records which rule)
        └─ background pass (UI, off the UI thread; mail/junk_pass.rs): batches
           of 250 until nothing is due, then idle until mail changes; a failed
           batch is retried after 30 s. Store::assess_pending
              ├─ junk::analyze  → warning signs and features (cached in
              │                   junk_assessments, keyed on the analysis
              │                   version and the contacts / accounts revision)
              └─ active model?  → score (redone alone when only the model changed)
reading pane / list: Store::assessment / advice_for
   advice = Phishing  (a Danger sign)
          | LooksLikeJunk (score ≥ threshold, Inbox, no label from you, no allow rule, suggestions on)
          | Caution  (a Caution sign)
          | None
```

Report junk / Report phishing / Not junk → `Store::report_junk` /
`Store::mark_not_junk`: the label (one effective label per message content,
history kept), any block rule and the move, in **one transaction**; Undo is
`Store::undo_junk_operation`, also one transaction → the label revision changes
→ the background pass calls `Store::retrain_if_needed`.

**When a cached check is redone.** Triggers drop a message's cached check when
its sender, subject, reply-to, authentication header, body or attachments
change; adding, changing or removing a contact, an address you wrote to or an
account bumps a revision every cached check is keyed on. A cached check is
never shown for content it didn't see. Read state, flags and moves don't
count. A check that can't finish is recorded as *incomplete* (the reading pane
says so), never as clean, and the other messages are still checked.

## Rule precedence

For a sender in an account:

1. An exact-address rule beats any domain rule.
2. Among domain rules (a domain covers its subdomains, matched on label
   boundaries; rules carried over from 0.1.4 cover that domain only), the most
   specific domain wins.
3. At the same level, the account's own rule beats an all-accounts rule.
4. With no rule, the advice above applies.

Each account and pattern has one rule, so a new choice replaces the old one.
Undo puts the old one back.

- **Block** moves new mail to Junk Email: mail that arrived after blocking took
  effect, with a day of slack for clocks. That time is kept per rule
  (`effective_at`): it is *now* when you block, also when the rule was an Allow
  before; blocking again keeps it; Undo brings back the earlier rule with its
  own. Mail you already have stays where it is, and so does mail a first sync
  brings in from before. An arrival time more than a day in the future isn't a
  server's (it came from the sender's Date header) and doesn't count as new.
- **Allow** ("Never block sender") overrides block rules for wider patterns and
  hides junk suggestions.

**Neither rule clears warning signs.** A known sender can be spoofed or hacked,
so phishing warnings still show.

**Pictures are separate.** "Always show content from…" only allows pictures
(`junk_image_senders`). It doesn't create a rule or a label, and an allow rule
doesn't allow pictures.

**Lists from 0.1.4 and earlier.** On first start, blocked and safe senders
become all-accounts rules marked "from an earlier version", meaning exactly what
they meant before:

- a domain entry covers that domain only, not its subdomains (rules and picture
  permissions both);
- old safe senders keep their picture permission and gain nothing more;
- blocking by a carried-over rule takes effect at migration, since the old lists
  never moved mail on a server.

The old tables are left as they were. Old backups restore the same way. Choosing
a carried-over domain again (Block, Never block) keeps it "this domain only";
a domain you add new covers its subdomains, as Settings says.

Profiles written by the first 0.1.5 build are repaired once on open: carried-over
rules (marked in the database) become exact again and take effect from the
repair, and picture permissions that the still-present old safe-sender list
shows came from it become exact. Permissions you added yourself keep the reach
they were given.

## Labels

- Only Report junk, Report phishing and Not junk record a label.
  - A plain Restore from Deleted Items doesn't.
  - Opening a message doesn't.
  - A picture permission doesn't.
  - A suggestion never does.
- Repeating a label changes nothing. A different label replaces the effective
  one, and the old one stays in the history.
- Undo (the toast, or Ctrl+Z) puts back the previous label, the move and any
  rule made with the report, together. A rule you changed again since is left
  as you changed it. Rule changes in Settings (add, remove) have Undo too.
- The key is a content fingerprint (account, sender, subject, date and the first
  4 KB of the text, or of the HTML when there is no text), so it survives moves
  and a resync: a copy synced again under a new id has the label without
  another report. A message's label is always looked up by its account and
  content, never by its row id, so mail that later gets a deleted message's id
  doesn't inherit its label. It doesn't rely on Message-ID.
- Labels are not merged across accounts.
- **Report phishing no longer deletes.** The message goes to Junk Email with a
  red banner. Nothing is reported to Microsoft or anyone else.

## Warning signs (`junk::analyze`)

The HTML is scanned as text: never rendered, no resources loaded. Links are
parsed but never followed or looked up. Attachments are judged by name, declared
type and size, and archives are not opened.

Inputs are capped (512 KB of HTML, 64 KB of text, 2 KB of subject, 512 bytes
of display name, 100 attachments, 200 links, 4 KB of authentication header).
When a cap is hit, the result says only part of the message was checked. The
scan is linear in the input and cuts text on character boundaries.

Signs, with their severity:

| Sign | Severity |
| --- | --- |
| Link text shows one site, link goes to another | Danger |
| `javascript:` / `data:` / `vbscript:` / `file:` link | Danger |
| User name before `@` in a link (`https://bank@evil/`) | Danger |
| Look-alike domain (of a familiar domain or a common brand; also hyphen parts like `netfl1x-support`) | Danger |
| Brand impersonation (brand in the display name, and the brand or a look-alike in a domain that isn't theirs) | Danger |
| Display name is a different address (on another domain) | Danger |
| Display name is one of your contacts, the address isn't theirs | Danger |
| Runnable attachment (`.exe`, `.js`, `.docm`, `.iso`, …) or double extension (`.pdf.exe`) | Danger |
| Asks for a password, code, recovery phrase, card / bank details or gift-card codes **and** presses for time | Danger |
| Receiving server reports DMARC fail | Danger |
| Bare IP link, punycode / non-ASCII domain, brand in a domain, a familiar name in someone else's domain, Reply-To elsewhere, HTML attachment, mistyped attachment, SPF / DKIM fail, asks for secrets (no time pressure) | Caution |
| Archive attachment (contents not checked) | Info: no banner on its own |

"Asks for secrets" is English only. Sentences that reassure ("we'll never ask
for your password") don't count.

**Sender checks.** Only the physically top `Authentication-Results` header is
kept (the receiving server adds its own above anything the sender wrote). It is
believed only when its authserv-id is one the account's provider is known to
use, and only for providers known to remove forged headers claiming it (RFC 8601
§5): Gmail (`mx.google.com`), Fastmail (`*.messagingengine.com`) and Yahoo / AOL
(`*.yahoo.com`). A matching name alone proves nothing, so for any other server
nothing is believed. Anything not believed counts as *unknown*, never *pass*.
Microsoft (Graph) doesn't deliver headers, so Outlook.com mail has no
sender-check signs here; Microsoft's own filter already ran on the server.

## The learner (`junk::learn`)

- **Model.** Logistic regression over hashed features (2^20), trained with plain
  SGD in a fixed hash order: the same data always gives the same model.
- **Features.** Words of the subject and the first 16 KB of text; the sender's
  domain; warning signs by kind; an all-capitals subject; attachment
  extensions. There are no format features (HTML or not, link counts, "!"
  counts, top-level domain, empty sender name): in the public corpora they tell
  2002 mailing lists from 2002 spam, not junk from mail.
- **Stored features.** Each label keeps its features (versioned, no text) so a
  rebuild works after the message is gone.
- **Training.** Always a full rebuild from the current effective labels, never an
  in-place update.
- **Held-back reports.** Reports are grouped by campaign: reports whose features
  are the same (one mail re-sent with a new counter or date, the same mail in
  two accounts) are one group. About one group in five is held back for
  checking and never trained on or used for thresholds; a group is never split
  between training and checking.
- **Minimum.** Training waits for 5 junk and 5 not-junk groups: thirty copies of
  one campaign count once.

**Checks before a model is used** (`learn::gate`, the same rule every time):

1. The candidate is compared with the model in use; with none, with *rules only*
   (no suggestions).
2. It is tested on three sets:
   - the shipped regression fixtures (the 358-message screening benchmark)
   - the shipped fresh validation set
   - your held-back reports
3. On each set it may not:
   - flag any wanted message the reference didn't
   - miss any phishing the reference caught
   - catch fewer unwanted messages
4. A set without wanted, unwanted (and, for the shipped sets, phishing) examples
   is *insufficient evidence*, and the current model stays.

The threshold comes from development data only: a sample of the SpamAssassin
development split, the modern development set, and the wanted reports trained
on. It is set strictly above the highest-scoring wanted message, and at least
one logit above what a message with no known words scores. Those bounds are
only ever raised: when no score can clear them (scores saturate at 1), the
candidate suggests nothing and isn't used. Model files with non-finite numbers
don't load.

**What the check does and doesn't cover.** A model can only add or remove a
*junk suggestion*; phishing warnings come from the warning signs, which take
priority and no model can change. So the gate's "phishing" counts are phishing
messages the candidate would *suggest as junk*, not phishing warnings, and a
model passing says nothing about how many phishing messages get a warning (the
warning signs alone warn on 14 of 20 fresh and 10 of 23 benchmark phishing
messages; see the evaluation). The runtime gate compares the candidate with the
model in use (or rules only); it doesn't separately rerun a fixed first-release
reference, measure review volume, or check a time / memory budget. That is a
narrowing of the original "full pipeline gate" and needs an explicit decision.

**Undo and forgetting.** Every change below takes effect in the same
transaction as the change itself, before any retraining and across a restart.

- If a report a model learned from is undone or changed, every model that
  learned from it (the one in use and kept earlier versions) is marked
  *affected* and is never used again: back to rules only until a rebuilt
  candidate passes.
- Roll back returns to the previous approved version, never to an affected one.
- Forget learned data deletes reports, history, models and cached scores.
  Rules and picture permissions stay.
- Removing an account removes its rules and reports, and deletes every model
  (in use, kept or rejected) trained on any of them.
- A training run works on a snapshot without locks. Its result is written only
  if no label, account or active model changed meanwhile, together with
  "this revision is done", in one transaction; otherwise it is thrown away
  whole. So a run that was under way when you chose Forget can't bring
  anything back.

**What's stored where.**

- Everything above is in the profile's database, next to the mail.
- Backups include rules and picture permissions. They don't include reports,
  scores or models (test `junk_reports_and_learned_models_stay_out_of_backups`).
- Nothing in this feature uses the network.

## Packaging

- `crates/axentra-core/src/junk/gate_fixtures.bin` holds the checks:
  deflate-compressed JSON of feature indices and labels, about 284 KB, no
  message text.
- `crates/axentra-core/src/junk/base_model.bin` is empty on purpose: no shipped
  model passed (see below). When a better bootstrap is available, putting a
  model there makes it the starting point and reference.

## Evaluation (2026-10-27)

Full numbers: `docs/junk/evaluation-2026-10-27.json`, regenerated after the audit
fixes with analysis version 3: every count, threshold and model digest is the
same as the first run's (version 1), and the regenerated `gate_fixtures.bin` is
byte-identical to the shipped one. (Version 2 reworded the contact-name sign;
version 3 adds the input caps and the stricter sender-check trust, which the
evaluation doesn't exercise: it believes no authentication header.) To reproduce:

```
python3 -I tools/junk/prepare.py <dir with the three SpamAssassin archives> <benchmark evaluation.jsonl> pool.jsonl
cargo run --release -p axentra-sync --example junk_train -- pool.jsonl <benchmark evaluation.jsonl> tools/junk/modern-dev.json tools/junk/fresh-validation.json out/
```

### Data

| Set | Messages | Role |
| --- | --- | --- |
| SpamAssassin pool, train split | 3,124 (995 unwanted) | Bootstrap training |
| SpamAssassin pool, dev split | 562 (180 unwanted) | Thresholds only |
| Screening benchmark | 358 (135 unwanted, 23 phishing) | Regression checks; never trained on |
| `modern-dev.json` | 60 (35 unwanted, 20 phishing) | Development; see note |
| `fresh-validation.json` | 60 (35 unwanted, 20 phishing) | Final check; frozen, then rerun after the changes below |

- **SpamAssassin pool.** It comes from 20030228 easy_ham and hard_ham and from
  20050311 spam_2. The 300 benchmark messages, 19 near-duplicates of them and
  141 other duplicates or unreadable messages were removed. The pool is split
  by campaign, so near-duplicates never straddle the train and dev splits.
- **Screening benchmark.** It was reused before for model comparisons, so it's
  not an untouched holdout. Its results also informed the "asks for secrets"
  sign.
- **`modern-dev.json`.** It was written as a fresh set, but its first results
  were used to diagnose the bootstrap failure and to add general warning-sign
  rules.
- **`fresh-validation.json`.** It was written after every rule and setting was
  fixed and first run then. It is not an untouched one-shot result: two
  changes followed that first run (listed below) and the numbers were rerun;
  the audit of 27 October and the regeneration after its fixes are reruns
  too, not new validation data.
- **Synthetic sets.** Both are assistant-authored synthetic mail, not
  independent annotations, and they are small.

### Results

| Pipeline, set | Wanted mail with any banner | of which by the model | Unwanted flagged (red banner or suggestion) | Phishing with a phishing warning | Phishing with nothing at all |
| --- | --- | --- | --- | --- | --- |
| Rules + warning signs only, fresh validation | 1 / 25 (one yellow) | – | 14 / 35 | 14 / 20 | 2 / 20 |
| Rules + warning signs only, benchmark | 7 / 223 (2 red, 5 yellow) | – | 15 / 135 | 10 / 23 | 9 / 23 |
| SpamAssassin model, fresh validation | 10 / 25 | 9 | 31 / 35 | 14 / 20 | 0 / 20 |
| SpamAssassin model, benchmark | 19 / 223 | 13 | 107 / 135 | 10 / 23 | 5 / 23 |
| First personal model (simulated), fresh validation | 1 / 25 | 0 | 14 / 35 | 14 / 20 | 2 / 20 |
| First personal model (simulated), benchmark | 7 / 223 | 0 | 15 / 135 | 10 / 23 | 9 / 23 |

The phishing columns don't sum to the total: the rest got a yellow "be careful"
banner or only a junk suggestion. Neither counts as a phishing warning.

Checking a message takes 0.06 ms median and about 1 ms at p95 (features plus
score, release build, this laptop).

**SpamAssassin as a bootstrap did not pass.** Trained on the pool, it flagged
16 of 25 modern legitimate messages in the first final run: receipts, alerts,
newsletters and short personal notes. With the threshold rule below it still
flags 9 of 25, plus 13 of 223 legitimate messages in the benchmark.

Its heaviest weights were artifacts of the 2002 corpus: HTML at all, a `.com`
sender, no display name, and second-person words like "your" and "click". These
are rare in the old mailing-list ham and normal in today's transactional mail.
Dropping format features and stronger regularisation (L2 up to 1e-3) didn't
change that. The check rejected it (`gate_vs_rules_only`), so it is **not
shipped**. Axentra starts with rules and warning signs only.

**The first personal model is a simplified offline simulation.** The modern
development set stands in for 60 of your reports. Unlike the app, the
simulation (`junk_train.rs`) trains on all 60 without holding any back, uses
a 400-message sample of the SpamAssassin development split for its threshold,
and runs only the two shipped check sets, not a third set of held-back reports.
It is not proof that the same 60 reports would make the app use a model. Trained from those alone, it passed the checks against
rules only and flagged no legitimate message. It also suggested junk for **none**
of the unwanted messages in the fresh set or the benchmark: they share almost
no words with the 60 reports, and the threshold demands real evidence. So a
personal model only speaks up about mail like what you've reported: repeat
senders, campaigns, phrasing. It won't recognise new kinds of junk. The store
tests show it does suggest junk for such repeats. This is a simulation with
synthetic mail, not a measurement on your mail.

**Two changes after the first final run.** Neither was tuned on any result.
The numbers above are from the last run; earlier runs are described here.

1. The feature hash went from 2^18 to 2^20 slots. Collisions made the check
   reject sound personal models in tests.
2. A threshold must also be at least one logit above what a message with no
   known words scores. The test results varied with the held-back split
   because unrelated mail could cross a threshold set just above the dev set's
   top legitimate score.

Before the second change, the simulated personal model caught 15 of 35 fresh
unwanted messages (and 7 in the benchmark) with no legitimate flags. That was
largely because those scores sat right at the edge.

**Hypotheses, not measured:**

- how the model behaves on a real mailbox
- how many reports it takes there before a model passes
- whether the conservative threshold catches enough

## Not done / limits

- **No Qwen or other language-model adviser**: deferred. No LLM fine-tuning and
  no model packaging beyond the fixtures.
- **No sender checks for Outlook.com (Graph) mail**: it delivers no headers.
- **Warning signs**:
  - "Asks for secrets" is English only.
  - Archives are never opened.
  - Brand lists are short and fixed.
  - Domain comparison uses an approximate registrable-domain rule, not the
    public-suffix list.
- **Social-engineering text without technical signs is often missed.** For
  example: 6 of 20 phishing messages in the fresh set got no phishing warning.
- **A personal model only recognises mail like what you've reported.** New
  kinds of junk get no suggestion; see the evaluation.
- **Near-duplicates that differ in a word** are separate campaign groups, so
  such a pair can still sit on both sides of the held-back split.
- **No measured time or memory budget** for a check end to end. Inputs are
  capped and the scan is linear; a check that fails is marked incomplete.
- **A large IMAP message whose body wasn't downloaded** is checked on what is
  stored (headers and the placeholder text), and isn't marked as unchecked.
- **Fake-server coverage.** The Graph test covers a block moving new mail once
  and leaving older mail; the IMAP test covers older mail staying put and
  nothing being pushed for it. Another device moving a blocked message back,
  UIDVALIDITY resets, duplicate deltas and an interrupted move push are not
  covered yet.
- **Evaluation**:
  - The fresh sets are small and synthetic.
  - The benchmark had been seen before.
  - There's no prospective or time-separated holdout from real mail.
  - Counts for account, source or time groups weren't possible.
