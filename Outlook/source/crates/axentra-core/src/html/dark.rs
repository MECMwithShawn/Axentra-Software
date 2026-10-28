//! Dark-mode colour adaptation of sanitised email HTML, for display only
//! (replies, forwards, drafts and the stored message keep the original).
//!
//! Every colour the email sets is rewritten through [`super::color`]:
//! inline `style` declarations, the `bgcolor` and `color` attributes, and
//! the declarations inside `<style>` blocks (including `@media` blocks),
//! custom properties that hold a colour, and shadow colours. Property
//! names, selectors, `url()`s, `!important` and everything else are copied
//! byte for byte; a colour token is only ever replaced by `#rrggbb` or
//! `rgba(r, g, b, a)`, and the only declarations ever added are a `color`
//! (a plain link, text on what is kept as designed), and a light
//! `background-color` (or a round `radial-gradient()` one for an icon)
//! behind an image or a background picture, so the output holds nothing
//! the sanitiser didn't already accept.
//!
//! Text colours are checked against the background they sit on where the
//! HTML says what that is (the element's own inline, `bgcolor` or class
//! background, else the nearest ancestor's; within a style rule, the rule's
//! own background), else against [`color::default_text_backdrop`].
//!
//! What the light design depends on is kept:
//! - A background picture that will load (`data:`, or `http(s):` when
//!   remote content is shown) is light or dark in ways the HTML can't tell,
//!   so its element, and the text over it, keep their colours (a descendant
//!   with an opaque background of its own is adapted again, and so is the
//!   colour its text inherits). Rules for classes used only there are kept
//!   too; an element there whose class is also used elsewhere (so its rule
//!   is adapted) keeps the rule's colour. A picture with no colour of its
//!   own (a transparent wordmark) gets the light colour it was on.
//! - An image on a light background that turned dark (the white page,
//!   when nothing says otherwise, unless a background the adapter can't
//!   read may be dark) gets that light colour behind it as a plate (round
//!   for icons, painted under the image, not clipping it), so transparent
//!   dark logos stay visible, and its alt text the light design's colour.
//!   Spacers (sized or named like one, or with no alt text outside a link)
//!   get none.
//! - Text hidden by matching its background stays hidden, and light text
//!   faded on a light background stays the dimmest that reads; a border on
//!   a background that was already dark (a white outline on a dark hero)
//!   stays.
//! - On a background that was already dark, a vivid fill (a neon coupon, a
//!   magenta button) is an accent of a dark design and stays, with what is
//!   on it and the light design's text colour, unless a rule other than a
//!   lone `.class` one may colour text there (then it is adapted like a
//!   box); so do rules, dots and bars. On a light background, an empty
//!   rule, bar or dot painted dark is folded into the border band instead,
//!   so it stays visible the way a border would (a chart's bars whatever
//!   their height; not an empty cell of a band painted like its content
//!   cells, or a tile holding an icon).
//! - The sender's own `@media (prefers-color-scheme: dark)` rules are the
//!   dark design: they are copied as they are, and images they show or
//!   hide (a logo swapped for a white one) get no plate. An `!important`
//!   dark class background is what is painted, over an inline one.
//! - A plain link on a coloured cell where the frame's link colour doesn't
//!   read gets a link colour that does.
//!
//! Two passes: the style sheets and the element tree are scanned for class
//! backgrounds, pictures, where classes are used and what custom
//! properties colour, then the HTML is rewritten. Both are linear, mapped
//! colours are cached, so a large newsletter costs about as much as copying
//! it twice.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

use super::color::{self, Backdrop, Rgba, Role};

/// Rewrite the colours of sanitised email HTML for the dark message canvas.
/// `allow_remote` says whether remote pictures will load (they decide which
/// backgrounds are pictures).
pub fn adapt_for_dark(html: &str, allow_remote: bool) -> String {
    let mut a = Adapter { allow_remote, ..Adapter::default() };
    a.prepare(html);
    a.walk(html)
}

/// The sanitised HTML for a light frame, whatever the system prefers: the
/// frame's `prefers-color-scheme` follows the GTK theme, not the frame's
/// own colour scheme, so in a dark app the sender's dark-mode rules would
/// apply inside a light body. In the email's style sheets,
/// `(prefers-color-scheme: dark)` in an `@media` prelude becomes a feature
/// that never matches and `(prefers-color-scheme: light)` one that always
/// does; nothing else changes.
pub fn light_scheme_only(html: &str) -> Cow<'_, str> {
    let mut out = String::new();
    let mut last = 0;
    let mut i = 0;
    while let Some(at) = find_tag(html, "<style", i) {
        let Some(gt) = html[at..].find('>') else { break };
        let start = at + gt + 1;
        let end = find_end_tag(html, "style", start).unwrap_or(html.len());
        if let Some(css) = light_sheet(&html[start..end]) {
            out.push_str(&html[last..start]);
            out.push_str(&css);
            last = end;
        }
        i = end;
    }
    if last == 0 {
        return Cow::Borrowed(html);
    }
    out.push_str(&html[last..]);
    Cow::Owned(out)
}

/// [`light_scheme_only`] for one style sheet; `None` when nothing changed.
fn light_sheet(css: &str) -> Option<String> {
    let mut out = String::new();
    let mut last = 0;
    let mut p = 0;
    loop {
        let (end, term) = scan_item(css, p);
        let Some(term) = term else { break };
        if term == b'{' && strip_comments(&css[p..end]).trim_start().trim_start_matches("<!--").trim_start_matches("-->").trim_start().get(..6).is_some_and(|m| m.eq_ignore_ascii_case("@media")) {
            let mut o = p;
            while let Some(rel) = css[o..end].find('(') {
                let open = o + rel;
                let Some(close) = css[open..end].find(')').map(|c| open + c) else { break };
                if css[open + 1..close].contains('(') {
                    o = open + 1;
                    continue;
                }
                let feature: String = strip_comments(&css[open + 1..close]).chars().filter(|c| !c.is_ascii_whitespace()).collect::<String>().to_ascii_lowercase();
                let new = match feature.as_str() {
                    "prefers-color-scheme:dark" => Some("(max-width:0px)"),
                    "prefers-color-scheme:light" => Some("(min-width:0px)"),
                    _ => None,
                };
                if let Some(new) = new {
                    out.push_str(&css[last..open]);
                    out.push_str(new);
                    last = close + 1;
                }
                o = close + 1;
            }
        }
        p = end + 1;
    }
    (last > 0).then(|| {
        out.push_str(&css[last..]);
        out
    })
}

/// Elements with no end tag.
const VOID: [&str; 14] = ["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
/// Elements whose content is raw text (no tags inside).
const RAW: [&str; 7] = ["script", "textarea", "title", "xmp", "iframe", "noembed", "noframes"];
/// Nesting deeper than this in a style sheet is copied unchanged.
const MAX_CSS_DEPTH: usize = 16;
/// Functions nested deeper than this in a value are copied unchanged
/// (colour functions never nest that deep).
const MAX_FN_DEPTH: usize = 32;
/// A background at least this opaque hides what is under it.
const OPAQUE: f64 = 0.99;
/// A light fill at least this vivid (OKLCH chroma) on a dark design is an
/// accent (a neon coupon), not a content box.
const ACCENT_CHROMA: f64 = 0.08;
/// An empty element painted at least this far from its light background
/// is a mark (a rule, a bar, a dot).
const MARK_CONTRAST: f64 = 1.5;

/// What a property's colours paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prop {
    Text,
    Background,
    Border,
    /// `box-shadow`, `text-shadow`: adapted like backgrounds (light shadows
    /// darken), but they paint no backdrop.
    Shadow,
    /// `--name`: adapted by what the property is used for.
    Custom,
}

impl Prop {
    fn bit(self) -> u8 {
        1 << self as u8
    }
}

// Where a class's elements were found (scan bits).
/// Inside a background picture.
const CTX_PIC: u8 = 1;
/// Not inside a picture.
const CTX_PLAIN: u8 = 2;
/// On a background that was dark.
const CTX_DARK: u8 = 4;
/// On a light or unknown background.
const CTX_LIGHT: u8 = 8;

/// A class's background from the email's top-level rules.
#[derive(Clone, Copy, Default)]
struct ClassBg {
    /// The colour (alpha kept), when one is set, and with `!important`.
    color: Option<Rgba>,
    important: bool,
    /// The colour the sender's own dark-mode rules give it (kept as it is).
    dark: Option<Rgba>,
    dark_important: bool,
    /// A background picture that will load.
    picture: bool,
}

/// A class's text colours from the email's top-level `.class` rules.
#[derive(Clone, Copy, Default)]
struct ClassText {
    /// `color` and `-webkit-text-fill-color` (with `!important`).
    color: Option<(Rgba, bool)>,
    fill: Option<(Rgba, bool)>,
    /// The rule's place in the sheets (a later rule wins).
    order: usize,
}

/// What a rule tells the rewrite, applied once its selector can match.
#[derive(Default)]
struct Effect {
    links_global: bool,
    link_classes: Vec<String>,
    unknown_bg: bool,
    unknown_dark: bool,
    ambiguous: Vec<String>,
    loose_text: bool,
}

/// How an element's or rule's own background is adapted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fill {
    /// Light ones darken, dark ones stay ([`color::map_background`]).
    Map,
    /// As designed: an image's backing, an accent or a mark on a dark
    /// design, anything on a background kept light.
    Keep,
    /// A dark rule, bar or dot on a light background: folded into the
    /// border band ([`color::map_border`]), so it stays visible.
    Ink,
}

/// Where a declaration list is.
#[derive(Clone, Copy)]
enum Site {
    /// A style rule; `dark_ctx`: its elements sit on dark backgrounds.
    Rule { dark_ctx: bool },
    /// A `style` attribute: the backdrop of the element's parent, whether
    /// it is an image, its `width` and `height` attributes, and whether it
    /// is empty (see [`is_mark`]); `locked`: an `!important` sender
    /// dark-mode class background wins over its own (non-`!important`) one.
    Element { parent: Option<Backdrop>, img: bool, width: Option<Dim>, height: Option<Dim>, empty: bool, locked: bool },
}

/// Where a style rule is.
#[derive(Clone, Copy)]
struct Scope {
    /// Its rules always apply in the dark frame (the sheet's top level, or
    /// `@media (prefers-color-scheme: dark)`), so class backgrounds can be
    /// read from them.
    applies: bool,
    /// Inside the sender's own dark-mode rules.
    dark: bool,
}

/// A length from an attribute or a declaration.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Dim {
    Px(f64),
    Pct(f64),
}

/// An open element.
#[derive(Clone, Copy, Default)]
struct Frame {
    /// The backdrop its content sits on, when known.
    bd: Option<Backdrop>,
    /// Where the sender's dark-mode rules repaint it, the light-mode
    /// background it had (what an image there was designed on).
    light: Option<Rgba>,
    /// Inside a background picture (no opaque background since).
    picture: bool,
    /// Inside an element whose class the email's link rules style.
    link_styled: bool,
    /// Inside an element the sender's dark-mode rules show or hide (a logo
    /// swapped for a white one): its images are as the sender made them.
    swapped: bool,
    /// The text colour the light design gives its content (`None`: the
    /// frame's), and whether the dark frame paints that colour as it is
    /// over a picture (kept or pinned there) rather than an adapted one.
    ink: Option<Rgba>,
    ink_kept: bool,
    /// On a background the adapter can't read (`var()`): an image there
    /// may be on a dark background.
    unknown: bool,
    /// Inside a link.
    in_link: bool,
}

#[derive(Default)]
struct Adapter {
    allow_remote: bool,
    /// The first pass: record, don't rewrite.
    scanning: bool,
    cache: HashMap<(u32, Role, u64), Rgba>,
    class_bg: HashMap<String, ClassBg>,
    class_ctx: HashMap<String, u8>,
    /// Custom property → the `Prop` bits of the declarations using it.
    var_roles: HashMap<String, u8>,
    /// A rule colours every link (`a`, `a:link`, `td a`).
    links_global: bool,
    /// Classes whose descendants' links a rule colours (`.lnk a`).
    link_classes: HashSet<String>,
    /// A rule paints a background on elements the adapter can't tell
    /// (`td.cta a`, `.btn.primary`): text that looks hidden may not be.
    unknown_bg: bool,
    /// Such a background is dark or a picture: an element with no known
    /// background may not be on the white page.
    unknown_dark: bool,
    class_text: HashMap<String, ClassText>,
    /// Classes whose text colour other rules also set (`.hero .x`,
    /// `@media`): the cascade there is not followed.
    text_ambiguous: HashSet<String>,
    /// Rules seen so far (their order).
    rule_order: usize,
    /// Effects of rules with attribute selectors (`[data-ogsb] .btn`),
    /// applied only when every attribute they need is in the HTML.
    pending: Vec<(Vec<String>, Effect)>,
    /// Attributes those selectors need, and those found in the HTML.
    wanted_attrs: HashSet<String>,
    seen_attrs: HashSet<String>,
    /// Classes the sender's dark-mode rules show or hide (`display`).
    dark_swapped: HashSet<String>,
    /// A rule other than a lone `.class` one colours text (`p`, `a`,
    /// `.box td`): text on a fill kept as designed may get an adapted
    /// colour from it.
    loose_text: bool,
    /// Classes whose background the adapter can't read (`var()`).
    class_unknown: HashSet<String>,
    /// Backgrounds of elements with content (a band's cells): an empty
    /// bar painted the same is part of that band, not a chart mark.
    box_colors: HashSet<u32>,
}

fn pack(c: Rgba) -> u32 {
    u32::from_be_bytes(c.to_u8())
}

fn bd_key(bd: Option<Backdrop>) -> u64 {
    bd.map_or(0, |b| 1 << 48 | u64::from(b.guess) << 49 | u64::from(pack(b.orig) >> 8) << 24 | u64::from(pack(b.mapped) >> 8))
}

/// A background (original and mapped, alpha kept) painted over the backdrop
/// below it; an unknown backdrop is the white page the email was designed
/// for and, mapped, [`color::default_text_backdrop`]. Translucent over an
/// unknown backdrop, the original is a guess.
fn compose(orig: Rgba, mapped: Rgba, below: Option<Backdrop>) -> Backdrop {
    let (bo, bm) = below.map_or_else(|| (color::WHITE, color::default_text_backdrop()), |b| (b.orig, b.mapped));
    let guess = below.map_or(orig.a < 1.0, |b| b.guess && orig.a < 1.0);
    Backdrop { orig: if orig.a >= 1.0 { orig } else { orig.over(bo) }, mapped: if mapped.a >= 1.0 { mapped } else { mapped.over(bm) }, guess }
}

/// The white page an element with no known background sits on, and the
/// canvas it is in dark mode.
const PAGE: Backdrop = Backdrop::new(color::WHITE, color::DARK_CANVAS);

/// A vivid light fill: on a dark design, an accent.
fn is_accent(c: Rgba) -> bool {
    c.a >= OPAQUE && !color::is_dark(c) && c.chroma() >= ACCENT_CHROMA
}

/// What a declaration list paints and how it was rewritten.
#[derive(Default)]
struct Decls {
    /// The new text when something changed.
    new: Option<String>,
    /// Its own background (original, as painted in dark mode; alpha kept).
    own: Option<(Rgba, Rgba)>,
    /// That background is `!important`.
    own_important: bool,
    /// A background picture that will load.
    picture: bool,
    /// A background the adapter can't read (`var()` with no colour).
    unknown_paint: bool,
    /// It sets the text colour.
    sets_text: bool,
    /// `color` and `-webkit-text-fill-color` when each is one colour (with
    /// `!important`).
    text: Option<(Rgba, bool)>,
    fill: Option<(Rgba, bool)>,
    /// It sets a corner radius, `display`.
    radius: bool,
    display: bool,
    /// `width`, `height`.
    width: Option<Dim>,
    height: Option<Dim>,
}

impl Adapter {
    fn map(&mut self, c: Rgba, role: Role, backdrop: Option<Backdrop>) -> Rgba {
        let backdrop = if role == Role::Background { None } else { backdrop };
        let key = (pack(c), role, bd_key(backdrop));
        let unknown_bg = self.unknown_bg;
        // As written out, so a second pass composes the same backdrops.
        *self.cache.entry(key).or_insert_with(|| {
            // Text that seems to match its background is only taken as
            // hidden (or faded) when every background in the email is known.
            let backdrop = match backdrop {
                Some(b) if unknown_bg && role == Role::Text => Some(Backdrop { guess: true, ..b }),
                b => b,
            };
            color::written(color::map(c, role, backdrop))
        })
    }

    /// The mapped colour when it differs (to the byte) from the original.
    fn adapt(&mut self, c: Rgba, role: Role, backdrop: Option<Backdrop>) -> Option<Rgba> {
        let m = self.map(c, role, backdrop);
        (m.to_u8() != c.to_u8()).then_some(m)
    }

    fn mapped_bg(&mut self, c: Rgba) -> Rgba {
        self.map(c, Role::Background, None)
    }

    // ---- CSS values ----

    /// Rewrite the colours in one CSS value; `None` when nothing changed.
    fn value(&mut self, v: &str, role: Role, backdrop: Option<Backdrop>) -> Option<String> {
        rewrite_colors(v, &mut |c| self.adapt(c, role, backdrop).map(Rgba::to_css))
    }

    /// The background a background declaration paints (original, mapped):
    /// its lightest colour once mapped (for a gradient, the stop light text
    /// has the hardest time on).
    fn painted(&mut self, v: &str) -> Option<(Rgba, Rgba)> {
        let mut found: Vec<Rgba> = Vec::new();
        rewrite_colors(v, &mut |c| {
            found.push(c);
            None
        });
        found.into_iter().map(|c| (c, self.mapped_bg(c))).max_by(|a, b| a.1.luminance().total_cmp(&b.1.luminance()))
    }

    /// True when a background value holds a picture that will load.
    fn loads(&self, v: &str) -> bool {
        let lower = v.to_ascii_lowercase();
        let b = lower.as_bytes();
        let mut i = 0;
        while let Some(rel) = lower[i..].find("url(") {
            let at = i + rel;
            i = at + 4;
            if at > 0 && (is_ident_byte(b[at - 1]) || b[at - 1] == b'\\') {
                continue;
            }
            let arg = lower[i..].trim_start().trim_start_matches(['"', '\'']).trim_start();
            if arg.starts_with("data:") || self.allow_remote && (arg.starts_with("http:") || arg.starts_with("https:")) {
                return true;
            }
        }
        false
    }

    /// The role a custom property's colour plays: how the email uses it
    /// (`color: var(--ink)`), else what it is called. `None` when it is
    /// used both for text and for backgrounds (any mapping breaks one).
    fn custom_role(&self, name: &str) -> Option<Role> {
        let bits = self.var_roles.get(name).copied().unwrap_or(0);
        let (text, bg, border) = (Prop::Text.bit(), Prop::Background.bit() | Prop::Shadow.bit(), Prop::Border.bit());
        if bits & text != 0 && bits & bg != 0 {
            return None;
        }
        if bits & text != 0 {
            return Some(Role::Text);
        }
        if bits & border != 0 {
            return Some(Role::Border);
        }
        if bits & bg != 0 {
            return Some(Role::Background);
        }
        let n = name.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
        Some(if has(&["bg", "background", "surface", "canvas", "card", "panel", "fill", "paper", "shadow"]) {
            Role::Background
        } else if has(&["border", "divider", "outline", "stroke", "separator", "line", "rule"]) {
            Role::Border
        } else {
            Role::Text
        })
    }

    /// Rewrite a declaration list (a `style` attribute or a rule's body).
    /// `parent` is the backdrop under its own background (for an element,
    /// what its `bgcolor` and classes paint over its parent's); `site` says
    /// where it is, which decides how its own background is adapted
    /// ([`Fill`]). Nothing is rewritten when it paints a picture or while
    /// scanning.
    fn declarations(&mut self, css: &str, parent: Option<Backdrop>, site: Site) -> Decls {
        let decls = split_declarations(css);
        let props: Vec<(Option<Prop>, Cow<'_, str>)> = decls.iter().map(|d| prop_of(&css[d.name.clone()])).collect();
        let mut out = Decls::default();
        for (d, (p, name)) in decls.iter().zip(&props) {
            let value = &css[d.value.clone()];
            let important = || contains_ci(value, "!important");
            match p {
                Some(Prop::Background) => {
                    let loads = self.loads(value);
                    out.picture |= loads;
                    match self.painted(value) {
                        Some(c) => {
                            out.unknown_paint = false;
                            let important = important();
                            if important || !out.own_important {
                                out.own = Some(c);
                                out.own_important = important;
                            }
                        }
                        None => out.unknown_paint |= !loads && contains_ci(value, "var("),
                    }
                }
                Some(Prop::Text) if name == "color" || name == "-webkit-text-fill-color" => {
                    out.sets_text = true;
                    let one = is_one_color(value).then(|| strip_important(value)).and_then(color::parse).map(|c| (c, important()));
                    if name == "color" { out.text = one } else { out.fill = one }
                }
                None if name.contains("radius") => out.radius = true,
                None if name == "display" => out.display = true,
                None if name == "width" => out.width = parse_dim(strip_important(value)),
                None if name == "height" => out.height = parse_dim(strip_important(value)),
                _ => {}
            }
            if self.scanning
                && let Some(p) = p
                && *p != Prop::Custom
            {
                for v in var_names(value) {
                    *self.var_roles.entry(v.to_owned()).or_default() |= p.bit();
                }
            }
        }
        if self.scanning || out.picture {
            return out;
        }
        let fill = match (site, out.own) {
            (_, None) => Fill::Map,
            (Site::Rule { .. }, _) => Fill::Map,
            (Site::Element { img: true, .. }, _) => Fill::Keep,
            (Site::Element { parent, width, height, empty, .. }, Some((o, _))) => {
                let (w, h) = (out.width.or(width), out.height.or(height));
                self.fill(o, parent, is_mark(w, h, empty), is_bar(w, h, empty))
            }
        };
        out.own = out.own.map(|(o, m)| match fill {
            Fill::Map => (o, m),
            Fill::Keep => (o, o),
            Fill::Ink => (o, self.map(o, Role::Border, None)),
        });
        let dark_ctx = matches!(site, Site::Rule { dark_ctx: true });
        // Under a sender's `!important` dark background its own one is
        // not what is painted.
        let locked = matches!(site, Site::Element { locked: true, .. }) && !out.own_important;
        let own = out.own.filter(|_| !locked).map(|(o, m)| compose(o, m, parent));
        let text_bd = own.or(parent);
        let border_bd = match text_bd {
            Some(b) => Some(b),
            None if dark_ctx => Some(Backdrop::new(color::DARK_CANVAS, color::DARK_CANVAS)),
            None => None,
        };
        let mut s = String::new();
        let mut last = 0;
        for (d, (p, name)) in decls.iter().zip(&props) {
            let Some(p) = p else { continue };
            let value = &css[d.value.clone()];
            let new = match (p, fill) {
                // An empty mark has no text (its colour would only follow
                // the mark's treatment from pass to pass).
                (Prop::Text, _) if matches!(site, Site::Element { empty: true, .. }) => None,
                (Prop::Text, _) => self.value(value, Role::Text, text_bd),
                (Prop::Background | Prop::Shadow, Fill::Keep) => None,
                (Prop::Background, Fill::Ink) => self.value(value, Role::Border, None),
                (Prop::Background | Prop::Shadow, _) => self.value(value, Role::Background, None),
                (Prop::Border, _) => self.value(value, Role::Border, border_bd),
                (Prop::Custom, _) => match self.custom_role(name) {
                    Some(role) if is_one_color(value) => self.value(value, role, if role == Role::Border { border_bd } else { None }),
                    _ => None,
                },
            };
            if let Some(new) = new {
                s.push_str(&css[last..d.value.start]);
                s.push_str(&new);
                last = d.value.end;
            }
        }
        if last > 0 {
            s.push_str(&css[last..]);
            out.new = Some(s);
        }
        out
    }

    /// How an element's own background (original `own`) is adapted, on
    /// its parent's backdrop; `mark`: the element is an empty rule, bar or
    /// dot ([`is_mark`]); `bar`: a chart bar ([`is_bar`]), a mark unless
    /// its colour is a box's (an empty cell of a coloured band).
    fn fill(&self, own: Rgba, parent: Option<Backdrop>, mark: bool, bar: bool) -> Fill {
        // A mark is only told apart on a background a second pass sees
        // darkened: not on the unknown page, where a darkened light spacer
        // would look like a dark rule.
        let Some(p) = parent else { return Fill::Map };
        if p.kept() {
            Fill::Keep
        } else if color::is_dark(p.orig) {
            // A dark design: its accents and marks are as designed.
            if mark || bar || self.kept_accent(own, parent) { Fill::Keep } else { Fill::Map }
        } else if (mark || bar && !self.box_colors.contains(&pack(own))) && !p.guess && own.a >= OPAQUE && color::is_inky(own) && color::contrast(own, p.orig) >= MARK_CONTRAST {
            Fill::Ink
        } else {
            Fill::Map
        }
    }

    /// A vivid fill kept as an accent of a dark design (on `parent`): only
    /// where no rule may colour the text on it (that text keeps the light
    /// design's colours, see [`Self::tag`]).
    fn kept_accent(&self, own: Rgba, parent: Option<Backdrop>) -> bool {
        parent.is_some_and(|p| !p.kept() && color::is_dark(p.orig)) && is_accent(own) && !self.loose_text
    }

    // ---- style sheets ----

    fn stylesheet(&mut self, css: &str) -> Option<String> {
        let mut out = String::with_capacity(css.len() + 64);
        self.rules(css, 0, 0, &mut out, Scope { applies: true, dark: false });
        // Belt and braces: a rewrite can't end the <style> element early.
        (out != css && find_end_tag(&out, "style", 0).is_none()).then_some(out)
    }

    /// A rule list (the sheet, or a group rule's body from `start`, which
    /// is just after its `{`). Returns the position after the closing `}`.
    fn rules(&mut self, css: &str, start: usize, depth: usize, out: &mut String, scope: Scope) -> usize {
        let mut p = start;
        loop {
            let (end, term) = scan_item(css, p);
            match term {
                Some(b'{') => {
                    out.push_str(&css[p..=end]);
                    let prelude = &css[p..end];
                    p = if depth >= MAX_CSS_DEPTH {
                        let close = skip_block(css, end + 1);
                        out.push_str(&css[end + 1..close]);
                        close
                    } else if is_group_rule(prelude) {
                        let dark = is_dark_media(prelude);
                        self.rules(css, end + 1, depth + 1, out, Scope { applies: scope.applies && dark, dark: scope.dark || dark })
                    } else {
                        self.block(css, end + 1, depth + 1, out, prelude, scope)
                    };
                }
                Some(b'}') => {
                    out.push_str(&css[p..=end]);
                    p = end + 1;
                    if depth > 0 {
                        return p;
                    }
                }
                Some(_) => {
                    out.push_str(&css[p..=end]);
                    p = end + 1;
                }
                None => {
                    out.push_str(&css[p..]);
                    return css.len();
                }
            }
        }
    }

    /// A declaration block from `start` (just after its `{`); nested rules
    /// inside it are handled too. Returns the position after its `}`.
    fn block(&mut self, css: &str, start: usize, depth: usize, out: &mut String, prelude: &str, scope: Scope) -> usize {
        // Once per block, however many rules are nested in it.
        let sels = selectors(prelude);
        let mut seg = start;
        let mut p = start;
        loop {
            let (end, term) = scan_item(css, p);
            match term {
                Some(b';') => p = end + 1,
                Some(b'{') => {
                    self.flush(&sels, scope, &css[seg..p], out);
                    out.push_str(&css[p..=end]);
                    p = if depth >= MAX_CSS_DEPTH {
                        let close = skip_block(css, end + 1);
                        out.push_str(&css[end + 1..close]);
                        close
                    } else {
                        self.block(css, end + 1, depth + 1, out, &css[p..end], Scope { applies: false, ..scope })
                    };
                    seg = p;
                }
                Some(_) => {
                    self.flush(&sels, scope, &css[seg..end], out);
                    out.push('}');
                    return end + 1;
                }
                None => {
                    self.flush(&sels, scope, &css[seg..], out);
                    return css.len();
                }
            }
        }
    }

    /// One rule's declarations: recorded while scanning, rewritten after.
    fn flush(&mut self, sels: &[Sel], scope: Scope, decls: &str, out: &mut String) {
        if self.scanning {
            self.scan_rule(sels, scope, decls);
            out.push_str(decls);
            return;
        }
        // The sender's dark-mode rules are the dark design. A picture's
        // rules, and rules for classes only used over one, keep the colours
        // the picture was designed with.
        let picture_class = |c: &String| self.class_bg.get(c).is_some_and(|b| b.picture);
        let only_in = |sel: &Sel, want: u8, not: u8| sel.subject.iter().any(|c| self.class_ctx.get(c).is_some_and(|&b| b & want != 0 && b & not == 0));
        let keep = scope.dark || sels.iter().any(|s| s.classes.iter().any(picture_class)) || (!sels.is_empty() && sels.iter().all(|s| only_in(s, CTX_PIC, CTX_PLAIN)));
        let dark_ctx = !sels.is_empty() && sels.iter().all(|s| only_in(s, CTX_DARK, CTX_LIGHT));
        let new = if keep { None } else { self.declarations(decls, None, Site::Rule { dark_ctx }).new };
        out.push_str(new.as_deref().unwrap_or(decls));
    }

    fn scan_rule(&mut self, sels: &[Sel], scope: Scope, decls: &str) {
        let d = self.declarations(decls, None, Site::Rule { dark_ctx: false });
        self.rule_order += 1;
        let paints = d.picture || d.own.is_some_and(|(o, _)| o.a > 0.0);
        let sets_text = d.text.is_some() || d.fill.is_some();
        for s in sels {
            let mut e = Effect::default();
            let lone_class = scope.applies && !s.pseudo && !s.attr && s.subject.len() == 1;
            if d.unknown_paint && !s.dynamic {
                // Its elements may be dark: hidden-looking text may not be,
                // and images there get no plate.
                e.unknown_bg = true;
                if lone_class {
                    self.class_unknown.insert(s.subject[0].clone());
                } else {
                    e.unknown_dark = true;
                }
            }
            if d.sets_text && !s.dynamic && !scope.dark && !(sets_text && s.simple && scope.applies) {
                e.loose_text = true;
            }
            if s.link && d.sets_text && s.subject.is_empty() {
                if s.classes.is_empty() {
                    e.links_global = true;
                } else {
                    e.link_classes = s.classes.clone();
                }
            }
            if paints && !s.dynamic {
                if lone_class {
                    let c = self.class_bg.entry(s.subject[0].clone()).or_default();
                    c.picture |= d.picture;
                    if let Some((o, _)) = d.own {
                        let (color, important) = if scope.dark { (&mut c.dark, &mut c.dark_important) } else { (&mut c.color, &mut c.important) };
                        if d.own_important || !*important {
                            *color = Some(o);
                            *important = d.own_important;
                        }
                    }
                } else {
                    e.unknown_bg = true;
                    e.unknown_dark = d.picture || d.own.is_some_and(|(o, _)| o.a >= color::BG_MIN_ALPHA && color::is_dark(o));
                }
            }
            if scope.dark && d.display {
                self.dark_swapped.extend(s.subject.iter().cloned());
            }
            if sets_text && !s.dynamic {
                if let [class] = s.subject.as_slice()
                    && s.simple
                    && scope.applies
                    && !scope.dark
                {
                    self.class_text.insert(class.clone(), ClassText { color: d.text, fill: d.fill, order: self.rule_order });
                } else {
                    e.ambiguous = s.subject.clone();
                }
            }
            if s.attrs.is_empty() {
                self.apply(e);
            } else {
                self.wanted_attrs.extend(s.attrs.iter().cloned());
                self.pending.push((s.attrs.clone(), e));
            }
        }
    }

    fn apply(&mut self, e: Effect) {
        self.links_global |= e.links_global;
        self.link_classes.extend(e.link_classes);
        self.unknown_bg |= e.unknown_bg;
        self.unknown_dark |= e.unknown_dark;
        self.text_ambiguous.extend(e.ambiguous);
        self.loose_text |= e.loose_text;
    }

    // ---- HTML ----

    /// Scan the style sheets, then the element tree, for what the rewrite
    /// needs to know.
    fn prepare(&mut self, src: &str) {
        self.scanning = true;
        let mut i = 0;
        while let Some(at) = find_tag(src, "<style", i) {
            let Some(gt) = src[at..].find('>') else { break };
            let start = at + gt + 1;
            let end = find_end_tag(src, "style", start).unwrap_or(src.len());
            let mut sink = String::new();
            self.rules(&src[start..end], 0, 0, &mut sink, Scope { applies: true, dark: false });
            i = end;
        }
        self.walk(src);
        // Rules for attributes the HTML never has (`[data-ogsb]`, set by
        // Outlook's own dark mode) can't match here.
        for (attrs, e) in std::mem::take(&mut self.pending) {
            if attrs.iter().all(|a| self.seen_attrs.contains(a)) {
                self.apply(e);
            }
        }
        self.scanning = false;
    }

    fn walk(&mut self, src: &str) -> String {
        let b = src.as_bytes();
        let scanning = self.scanning;
        let mut out = String::with_capacity(if scanning { 0 } else { src.len() + src.len() / 16 });
        // Open elements and what their content sits on.
        let mut stack: Vec<(String, Frame)> = Vec::new();
        let mut last = 0;
        let mut i = 0;
        while let Some(rel) = src[i..].find('<') {
            let lt = i + rel;
            let next = b.get(lt + 1).copied().unwrap_or(0);
            if src[lt..].starts_with("<!--") {
                i = src[lt + 4..].find("-->").map_or(src.len(), |e| lt + 4 + e + 3);
                continue;
            }
            if next == b'!' || next == b'?' {
                i = src[lt..].find('>').map_or(src.len(), |e| lt + e + 1);
                continue;
            }
            if next == b'/' && b.get(lt + 2).is_some_and(u8::is_ascii_alphabetic) {
                let gt = src[lt..].find('>').map_or(src.len(), |e| lt + e + 1);
                let name: String = src[lt + 2..gt].chars().take_while(|c| !c.is_ascii_whitespace() && *c != '>' && *c != '/').collect::<String>().to_ascii_lowercase();
                if let Some(pos) = stack.iter().rposition(|(n, _)| *n == name) {
                    stack.truncate(pos);
                }
                i = gt;
                continue;
            }
            if !next.is_ascii_alphabetic() {
                i = lt + 1;
                continue;
            }
            let Some(tag) = parse_tag(src, lt) else { break };
            let name = src[tag.name.clone()].to_ascii_lowercase();
            let parent = stack.last().map(|(_, f)| *f).unwrap_or_default();
            let (edits, frame) = self.tag(src, &tag, &name, &parent);
            if !edits.is_empty() {
                out.push_str(&src[last..lt]);
                let mut p = lt;
                for (range, text) in edits {
                    out.push_str(&src[p..range.start]);
                    out.push_str(&text);
                    p = range.end;
                }
                out.push_str(&src[p..tag.end]);
                last = tag.end;
            }
            i = tag.end;
            if !tag.self_closing && !VOID.contains(&name.as_str()) {
                stack.push((name.clone(), frame));
            }
            if name == "style" || RAW.contains(&name.as_str()) {
                let close = find_end_tag(src, &name, i).unwrap_or(src.len());
                if name == "style"
                    && !scanning
                    && let Some(css) = self.stylesheet(&src[i..close])
                {
                    out.push_str(&src[last..i]);
                    out.push_str(&css);
                    last = close;
                }
                i = close;
            }
        }
        if !scanning {
            out.push_str(&src[last..]);
        }
        out
    }

    /// The edits for one start tag (range, replacement text) and the frame
    /// its content sits in.
    fn tag(&mut self, src: &str, tag: &Tag, name: &str, parent: &Frame) -> (Vec<(Range<usize>, String)>, Frame) {
        let attr = |n: &str| tag.attrs.iter().find(|a| src[a.name.clone()].eq_ignore_ascii_case(n));
        if self.scanning && !self.wanted_attrs.is_empty() {
            for a in &tag.attrs {
                let n = src[a.name.clone()].to_ascii_lowercase();
                if self.wanted_attrs.contains(&n) {
                    self.seen_attrs.insert(n);
                }
            }
        }
        let mut edits = Vec::new();
        let class_attr = attr("class").map(|a| decode_attr(&src[a.value.clone()]).into_owned()).unwrap_or_default();
        let classes: Vec<&str> = class_attr.split_ascii_whitespace().collect();
        let is_img = name == "img";
        let dim = |n: &str| attr(n).and_then(|a| parse_dim(&decode_attr(&src[a.value.clone()])));
        let (width, height) = (dim("width"), dim("height"));
        let style = attr("style").map(|a| (a, decode_attr(&src[a.value.clone()]).into_owned()));
        let bgcolor = attr("bgcolor").and_then(|a| color::parse_legacy(&decode_attr(&src[a.value.clone()])).map(|c| (a, c)));
        // Only a painted element can be a mark.
        let empty = !is_img && (bgcolor.is_some() || style.as_ref().is_some_and(|(_, v)| contains_ci(v, "background"))) && is_empty_element(src, tag.end);
        // Its own background, lowest priority first: bgcolor, class rules,
        // the style attribute.
        let mut bd = parent.bd;
        let mut opaque = false;
        let mut picture = false;
        let mut bgcolor_mapped = None;
        // Over a picture (kept as designed), what is under an element is
        // not known to have darkened.
        let fill_parent = if parent.picture { None } else { parent.bd };
        if let Some((_, c)) = bgcolor {
            let (mark, bar) = if empty {
                let (sw, sh) = style.as_ref().map_or((None, None), |(_, v)| dims_of(v));
                let (w, h) = (sw.or(width), sh.or(height));
                (is_mark(w, h, true), is_bar(w, h, true))
            } else {
                (false, false)
            };
            let m = match if self.scanning { Fill::Map } else { self.fill(c, fill_parent, mark, bar) } {
                Fill::Map => self.mapped_bg(c),
                Fill::Keep => c,
                Fill::Ink => self.map(c, Role::Border, None),
            };
            bd = Some(Backdrop::new(c, m));
            bgcolor_mapped = Some(m);
            opaque = true;
        }
        let mut light = parent.light.filter(|_| bgcolor.is_none());
        // A sender's dark-mode class background (`!important`, the frame
        // paints it whatever the element's own style says).
        let mut sender_dark = false;
        let mut locked = false;
        for c in &classes {
            if let Some(cb) = self.class_bg.get(*c).copied() {
                picture |= cb.picture;
                // The sender's dark-mode colour, unless a light rule wins
                // with `!important`: that is what the frame paints.
                match (cb.dark.filter(|_| cb.dark_important || !cb.important), cb.color) {
                    (Some(d), col) => {
                        let under = light.or(bd.map(|b| b.orig)).unwrap_or(color::WHITE);
                        light = Some(col.map_or(under, |col| col.over(under)));
                        // With `!important` it is painted whatever the light
                        // design had there (an inline white): what text
                        // matched there is no guide to hidden text.
                        let b = compose(d, d, bd);
                        bd = Some(Backdrop { guess: b.guess || cb.dark_important, ..b });
                        opaque |= d.a >= OPAQUE;
                        sender_dark = true;
                        locked |= cb.dark_important;
                    }
                    (None, Some(col)) => {
                        let m = self.mapped_bg(col);
                        bd = Some(compose(col, m, bd));
                        if col.a >= OPAQUE {
                            opaque = true;
                            light = None;
                        }
                    }
                    (None, None) => {}
                }
            }
        }
        let decls = match &style {
            Some((_, v)) => self.declarations(v, bd, Site::Element { parent: fill_parent, img: is_img, width, height, empty, locked }),
            None => Decls::default(),
        };
        picture |= decls.picture;
        if let Some((o, m)) = decls.own {
            if locked && !decls.own_important {
                // The sender's dark colour is painted; its own one is what
                // light mode showed (what an image there was designed on).
                light = Some(if o.a >= OPAQUE { o } else { o.over(light.unwrap_or(color::WHITE)) });
            } else {
                bd = Some(compose(o, m, bd));
                if o.a >= OPAQUE {
                    opaque = true;
                    light = None;
                }
            }
        }
        let own_picture = picture;
        let picture = picture || parent.picture && !opaque;
        let unknown_here = decls.unknown_paint || classes.iter().any(|c| self.class_unknown.contains(*c));
        let mut frame = Frame {
            bd,
            light,
            picture,
            link_styled: parent.link_styled || classes.iter().any(|c| self.link_classes.contains(*c)),
            swapped: parent.swapped || classes.iter().any(|c| self.dark_swapped.contains(*c)),
            ink: parent.ink,
            ink_kept: parent.ink_kept,
            unknown: unknown_here || parent.unknown && !opaque,
            in_link: parent.in_link || name == "a",
        };
        if self.scanning {
            let outside = if parent.bd.is_some_and(|b| color::is_dark(b.orig)) { CTX_DARK } else { CTX_LIGHT };
            let here = if picture { CTX_PIC } else { CTX_PLAIN };
            for c in &classes {
                *self.class_ctx.entry((*c).to_owned()).or_default() |= outside | here;
            }
            // A background of an element with content (not a picture's:
            // that may get a plate).
            if !empty && !is_img && !picture {
                for c in [bgcolor.map(|(_, c)| c), decls.own.map(|(o, _)| o)].into_iter().flatten() {
                    self.box_colors.insert(pack(c));
                }
            }
            return (edits, frame);
        }
        // The light design's text colour here, and whether the dark frame
        // paints it as it is over a picture (where a box with its own
        // background is adapted again, see below).
        let kept_here = picture || bd.is_some_and(|b| b.kept());
        let class_text = classes.iter().filter_map(|c| self.class_text.get(*c).map(|t| (*c, *t))).max_by_key(|(_, t)| t.order);
        let class_colored = class_text.is_some() || classes.iter().any(|c| self.text_ambiguous.contains(*c));
        if name == "font"
            && let Some(c) = attr("color").and_then(|a| color::parse_legacy(&decode_attr(&src[a.value.clone()])))
        {
            (frame.ink, frame.ink_kept) = (Some(c), picture);
        }
        if class_colored {
            let c = class_text.filter(|(c, _)| !self.text_ambiguous.contains(*c)).and_then(|(_, t)| t.color).map(|(c, _)| c);
            (frame.ink, frame.ink_kept) = (c, picture && c.is_some());
        }
        if decls.sets_text {
            (frame.ink, frame.ink_kept) = (decls.text.map(|(c, _)| c), picture && decls.text.is_some());
        }
        // What to add: a light plate behind an image on a background that
        // turned dark; a readable colour on a plain link; over a picture or
        // on a background kept light, the colour of a class rule that was
        // adapted for its other uses.
        let mut add: Vec<String> = Vec::new();
        let mut link_added = false;
        if name == "a" && !decls.sets_text && !class_colored {
            // A link's content is in the light design's link colour.
            (frame.ink, frame.ink_kept) = (Some(color::LIGHT_LINK), false);
        }
        if is_img && !picture {
            if !frame.swapped {
                add.extend(self.backing(src, tag, parent, &decls, width, height).filter(|_| !locked));
            }
        } else if name == "a" && !picture && !empty && classes.is_empty() && !decls.sets_text {
            add.extend(self.link_color(&frame));
            link_added = !add.is_empty();
        } else if kept_here && !decls.sets_text {
            add.extend(self.pinned_text(&classes));
        }
        if !is_img {
            let colored = decls.sets_text || class_colored || !add.is_empty() || empty;
            let light_ink = parent.ink.unwrap_or(color::LIGHT_TEXT);
            // The accent the content sits on (the top background).
            let accent = [bgcolor.map(|(_, c)| c), decls.own.map(|(o, _)| o)].into_iter().flatten().rfind(|c| self.kept_accent(*c, fill_parent));
            let accent = !empty && !picture && accent.is_some_and(|a| bd.is_some_and(|b| b.kept() && b.orig.to_u8() == a.to_u8()));
            // A background picture with no colour under it (a transparent
            // wordmark, an icon) was designed on the light colour below:
            // it gets that as a plate, and text there its light colour.
            if own_picture
                && !parent.picture
                && !frame.swapped
                && !frame.unknown
                && bgcolor.is_none()
                && decls.own.is_none()
                && !classes.iter().any(|c| self.class_bg.get(*c).is_some_and(|b| b.color.is_some() || b.dark.is_some()))
                && let Some(plate) = self.plate(parent)
            {
                add.push(format!("background-color:{}", plate.to_hex()));
                if !colored {
                    add.push(format!("color:{}", light_ink.to_hex()));
                }
            } else if !colored && accent && !sender_dark {
                // An accent kept on a dark design: what is on it keeps the
                // light design's text colour, not the dark frame's.
                add.push(format!("color:{}", light_ink.to_hex()));
            } else if !colored
                && !picture
                && opaque
                && parent.ink_kept
                && let Some(c) = parent.ink
                && let Some(m) = self.adapt(c, Role::Text, bd)
            {
                // A box adapted again under a colour kept as designed (over
                // a picture): its text gets that colour adapted.
                add.push(format!("color:{}", m.to_hex()));
            }
        }
        // A colour added here is the content's, as a second pass reads it
        // (a link's stays the light design's link colour).
        let added_ink = if is_img || link_added { None } else { add.iter().rev().find_map(|a| a.strip_prefix("color:")).and_then(color::parse) };
        let add = (!add.is_empty()).then(|| add.join(";"));
        if !picture
            && let (Some((a, c)), Some(m)) = (bgcolor, bgcolor_mapped)
            && m.to_u8() != c.to_u8()
        {
            edits.push((a.token.clone(), format!("\"{}\"", m.to_hex())));
        }
        // A picture, or content over one, is otherwise as designed.
        let new = if picture { None } else { decls.new };
        let new_style = match (new, add) {
            (new, Some(add)) => {
                let v = new.unwrap_or_else(|| style.as_ref().map(|(_, v)| v.clone()).unwrap_or_default());
                // Only where the browser reads it as declarations of their
                // own (not inside an unclosed string, comment or bracket).
                match append_decls(&v, &add) {
                    Some(v) => {
                        if let Some(c) = added_ink {
                            (frame.ink, frame.ink_kept) = (Some(c), picture);
                        }
                        Some(v)
                    }
                    None => (v != style.as_ref().map(|(_, v)| v.as_str()).unwrap_or_default()).then_some(v),
                }
            }
            (new, None) => new,
        };
        if let Some(v) = new_style {
            match &style {
                Some((a, _)) => edits.push((a.token.clone(), format!("\"{}\"", escape_attr(&v)))),
                None => edits.push((tag.name.end..tag.name.end, format!(" style=\"{}\"", escape_attr(&v)))),
            }
        }
        if !picture
            && !empty
            && let Some(a) = attr("color")
            && let Some(c) = color::parse_legacy(&decode_attr(&src[a.value.clone()]))
            && let Some(m) = self.adapt(c, Role::Text, bd)
        {
            edits.push((a.token.clone(), format!("\"{}\"", m.to_hex())));
        }
        edits.sort_by_key(|(r, _)| r.start);
        (edits, frame)
    }

    /// The light colour under an element on a light background that
    /// turned dark (the white page when nothing says otherwise; the
    /// light-mode one where the sender's dark-mode rules repaint it): what
    /// a picture there was designed on. `None` on a dark, kept or unknown
    /// background.
    fn plate(&self, parent: &Frame) -> Option<Rgba> {
        if parent.unknown {
            return None;
        }
        let bd = match parent.bd {
            Some(b) => b,
            None if self.unknown_dark => return None,
            None => PAGE,
        };
        // As written out (what a second pass reads back).
        let light = color::written(Rgba { a: 1.0, ..parent.light.unwrap_or(bd.orig) });
        (!color::is_dark(light) && light.to_u8() != bd.mapped.to_u8()).then_some(light)
    }

    /// The light plate for an image that will load and has no background
    /// of its own ([`Self::plate`]), so a transparent dark logo stays
    /// visible: icons get a round one (painted, not clipped: a square icon
    /// keeps its corners), and the alt text of an image that fails to load
    /// the light design's colour. Spacers get none.
    fn backing(&self, src: &str, tag: &Tag, parent: &Frame, decls: &Decls, width: Option<Dim>, height: Option<Dim>) -> Option<String> {
        if decls.own.is_some() {
            return None;
        }
        let light = self.plate(parent)?;
        let attr = |n: &str| tag.attrs.iter().find(|a| src[a.name.clone()].eq_ignore_ascii_case(n)).map(|a| decode_attr(&src[a.value.clone()]).trim().to_ascii_lowercase());
        let src_url = attr("src")?;
        if !(src_url.starts_with("data:") || self.allow_remote && (src_url.starts_with("http:") || src_url.starts_with("https:"))) {
            return None;
        }
        let px = |d: Option<Dim>| match d {
            Some(Dim::Px(v)) => Some(v),
            _ => None,
        };
        let (w, h) = (px(decls.width.or(width)), px(decls.height.or(height)));
        // Spacers and pixels: an image named or sized like one, or one with
        // no alt text that is not a link.
        if w.is_some_and(|v| v <= 2.0) || h.is_some_and(|v| v <= 2.0) || is_spacer(src, tag) || attr("alt").is_none_or(|a| a.is_empty()) && !parent.in_link {
            return None;
        }
        let plate = light.to_hex();
        let mut out = match (w, h) {
            (Some(w), Some(h)) if !decls.radius && w <= 48.0 && h <= 48.0 && (0.75..=1.34).contains(&(w / h)) => format!("background-image:radial-gradient(circle closest-side,{plate} 97%,transparent 100%)"),
            _ => format!("background-color:{plate}"),
        };
        // Its own colour (adapted for the dark backdrop) or the inherited one
        // would be the alt text's: the light design's goes on the plate.
        match (decls.sets_text, decls.text) {
            (false, _) => out.push_str(&format!(";color:{}", parent.ink.unwrap_or(color::LIGHT_TEXT).to_hex())),
            (true, Some((c, important))) => out.push_str(&format!(";color:{}{}", c.to_css(), if important { " !important" } else { "" })),
            (true, None) => {}
        }
        Some(out)
    }

    /// `color` for a link with no colour of its own (none set, no class,
    /// no link rule for it) on a known background where the frame's link
    /// colour doesn't read.
    fn link_color(&mut self, frame: &Frame) -> Option<String> {
        if self.links_global || frame.link_styled {
            return None;
        }
        let bd = frame.bd?;
        let mapped = color::written(bd.mapped);
        if color::contrast(color::DARK_LINK, mapped) >= color::MIN_CONTRAST {
            return None;
        }
        // On a background kept light, the light design's link colour.
        if bd.kept() {
            return Some(format!("color:{}", color::LIGHT_LINK.to_hex()));
        }
        let c = self.map(color::DARK_LINK, Role::Text, Some(Backdrop::new(color::WHITE, bd.mapped)));
        Some(format!("color:{}", c.to_hex()))
    }

    /// Over a picture or on a background kept light: `color` (and
    /// `-webkit-text-fill-color`) of the winning `.class` rule when that
    /// rule is adapted for the class's other uses, so the text keeps the
    /// colour it was designed with there.
    fn pinned_text(&mut self, classes: &[&str]) -> Option<String> {
        let (class, t) = classes.iter().filter_map(|c| self.class_text.get(*c).map(|t| (*c, *t))).max_by_key(|(_, t)| t.order)?;
        let ctx = self.class_ctx.get(class).copied().unwrap_or(0);
        if self.text_ambiguous.contains(class) || ctx & CTX_PLAIN == 0 || self.class_bg.get(class).is_some_and(|b| b.picture) {
            return None;
        }
        let mut out = Vec::new();
        for (prop, v) in [("color", t.color), ("-webkit-text-fill-color", t.fill)] {
            if let Some((c, important)) = v
                && self.adapt(c, Role::Text, None).is_some()
            {
                out.push(format!("{prop}:{}{}", c.to_css(), if important { " !important" } else { "" }));
            }
        }
        (!out.is_empty()).then(|| out.join(";"))
    }
}

/// A selector of a rule's prelude, as far as the adapter cares.
#[derive(Default)]
struct Sel {
    /// Classes on the subject (the last compound).
    subject: Vec<String>,
    /// Classes elsewhere (ancestors and siblings).
    classes: Vec<String>,
    /// The subject has a pseudo-class or pseudo-element.
    pseudo: bool,
    /// Only while hovered, pressed or focused.
    dynamic: bool,
    /// The subject is a link at rest (`a`, `:link`, `:visited`).
    link: bool,
    /// It has an attribute selector (`[data-ogsb] .x`: may not match).
    attr: bool,
    /// The attributes those need (lowercase).
    attrs: Vec<String>,
    /// It is a lone class (`.x`).
    simple: bool,
}

/// The selectors of a rule prelude (none for at-rules).
fn selectors(prelude: &str) -> Vec<Sel> {
    let p = strip_comments(prelude);
    let p = p.trim();
    if p.starts_with('@') || p.is_empty() {
        return Vec::new();
    }
    let b = p.as_bytes();
    let mut out = Vec::new();
    let mut sel = Sel::default();
    // The current compound: its classes, tag, pseudo-classes.
    let mut compound: Vec<String> = Vec::new();
    let mut tag = String::new();
    let mut pseudo = String::new();
    let mut depth = 0usize;
    let mut i = 0;
    let mut sel_start = 0;
    let finish = |sel: &mut Sel, compound: &mut Vec<String>, tag: &mut String, pseudo: &mut String, raw: &str| {
        let raw = raw.trim();
        sel.simple = raw.len() > 1 && raw.starts_with('.') && ident_end(raw.as_bytes(), 1) == raw.len();
        sel.classes.append(&mut sel.subject);
        sel.subject.append(compound);
        let dynamic = [":hover", ":active", ":focus"].iter().any(|d| pseudo.contains(d));
        sel.pseudo = !pseudo.is_empty();
        sel.dynamic = dynamic;
        sel.link = !dynamic && (tag == "a" || [":link", ":visited", ":any-link"].iter().any(|l| pseudo.contains(l)));
        tag.clear();
        pseudo.clear();
    };
    while i < b.len() {
        let c = b[i];
        match c {
            b'(' | b'[' => {
                if c == b'[' && depth == 0 {
                    sel.attr = true;
                    let mut s = i + 1;
                    while s < b.len() && b[s].is_ascii_whitespace() {
                        s += 1;
                    }
                    let mut e = ident_end(b, s);
                    // A namespace prefix (`ns|name`, `*|name`).
                    if b.get(s) == Some(&b'*') || b.get(e) == Some(&b'|') && b.get(e + 1) != Some(&b'=') {
                        s = (e.max(s + 1) + 1).min(b.len());
                        e = ident_end(b, s);
                    }
                    sel.attrs.push(css_unescape(&p[s..e]).to_ascii_lowercase());
                }
                depth += 1;
                i += 1;
            }
            b')' | b']' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ if depth > 0 => i += 1,
            b',' => {
                finish(&mut sel, &mut compound, &mut tag, &mut pseudo, &p[sel_start..i]);
                out.push(std::mem::take(&mut sel));
                i += 1;
                sel_start = i;
            }
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | b'>' | b'+' | b'~' => {
                // A combinator: the compound so far is not the subject (white
                // space before a `,`, the end or another combinator is none).
                // (The whole run of white space at once: linear.)
                let run = if c.is_ascii_whitespace() { b[i..].iter().take_while(|c| c.is_ascii_whitespace()).count() } else { 1 };
                let combinator = !c.is_ascii_whitespace() || b.get(i + run).is_some_and(|n| !matches!(n, b',' | b'>' | b'+' | b'~'));
                if combinator && (!compound.is_empty() || !tag.is_empty() || !pseudo.is_empty()) {
                    sel.classes.append(&mut compound);
                    tag.clear();
                    pseudo.clear();
                }
                i += run;
            }
            b'.' => {
                let end = ident_end(b, i + 1);
                compound.push(p[i + 1..end].to_owned());
                i = end.max(i + 1);
            }
            b':' => {
                let end = ident_end(b, i + 1 + usize::from(b.get(i + 1) == Some(&b':')));
                pseudo.push_str(&p[i..end].to_ascii_lowercase());
                i = end.max(i + 1);
            }
            _ if is_ident_byte(c) || c == b'*' => {
                let end = ident_end(b, i).max(i + 1);
                tag = p[i..end].to_ascii_lowercase();
                i = end;
            }
            _ => i += 1,
        }
    }
    finish(&mut sel, &mut compound, &mut tag, &mut pseudo, &p[sel_start..]);
    out.push(sel);
    out
}

fn ident_end(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (is_ident_byte(b[i]) || b[i] == b'\\') {
        i += if b[i] == b'\\' { 2 } else { 1 };
    }
    i.min(b.len())
}

/// The text without CSS comments and `<!--` / `-->`.
fn strip_comments(s: &str) -> Cow<'_, str> {
    if !s.contains("/*") && !s.contains("<!--") && !s.contains("-->") {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let b = s.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            out.push_str(&s[last..i]);
            i = skip_comment(b, i);
            last = i;
        } else if b[i..].starts_with(b"<!--") || b[i..].starts_with(b"-->") {
            out.push_str(&s[last..i]);
            i += if b[i] == b'<' { 4 } else { 3 };
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[last..]);
    Cow::Owned(out)
}

/// Decode CSS escapes in an identifier (`\63 olor` is `color`).
fn css_unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('\\') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let mut hex = String::new();
        while hex.len() < 6
            && let Some(&(_, h)) = chars.peek()
            && h.is_ascii_hexdigit()
        {
            hex.push(h);
            chars.next();
        }
        if hex.is_empty() {
            if let Some((_, n)) = chars.next() {
                out.push(n);
            }
        } else {
            if chars.peek().is_some_and(|&(_, w)| w.is_ascii_whitespace()) {
                chars.next();
            }
            let code = u32::from_str_radix(&hex, 16).unwrap_or(0xfffd);
            out.push(char::from_u32(code).filter(|&c| c != '\0').unwrap_or('\u{fffd}'));
        }
    }
    Cow::Owned(out)
}

/// What a property's colours paint, and its name as the browser reads it
/// (comments and `<!--` dropped, escapes decoded; lowercase unless custom).
fn prop_of(raw: &str) -> (Option<Prop>, Cow<'_, str>) {
    let name: Cow<'_, str> = match strip_comments(raw) {
        Cow::Borrowed(b) => css_unescape(b.trim()),
        Cow::Owned(o) => Cow::Owned(css_unescape(o.trim()).into_owned()),
    };
    if name.starts_with("--") {
        return (Some(Prop::Custom), name);
    }
    let n = name.to_ascii_lowercase();
    let p = match n.as_str() {
        "color" | "-webkit-text-fill-color" | "text-decoration" | "text-decoration-color" | "-webkit-text-stroke-color" => Prop::Text,
        "background" | "background-color" | "background-image" => Prop::Background,
        "box-shadow" | "-webkit-box-shadow" | "-moz-box-shadow" | "text-shadow" => Prop::Shadow,
        "outline" | "outline-color" | "column-rule" | "column-rule-color" => Prop::Border,
        _ if n.starts_with("border") && !n.contains("radius") && !n.contains("width") && !n.contains("style") && !n.contains("image") && !n.contains("collapse") && !n.contains("spacing") => Prop::Border,
        _ => return (None, Cow::Owned(n)),
    };
    (Some(p), Cow::Owned(n))
}

/// The custom properties a value reads (`var(--name ...)`).
fn var_names(v: &str) -> impl Iterator<Item = &str> {
    let lower = v.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i + 4 <= lower.len() {
            if lower[i..i + 4].eq_ignore_ascii_case(b"var(") {
                let mut j = i + 4;
                while j < lower.len() && lower[j].is_ascii_whitespace() {
                    j += 1;
                }
                let start = j;
                while j < lower.len() && (is_ident_byte(lower[j])) {
                    j += 1;
                }
                i = j.max(i + 4);
                if v[start..j].starts_with("--") {
                    return Some(&v[start..j]);
                }
            } else {
                i += 1;
            }
        }
        None
    })
}

/// The value without a trailing `!important`.
fn strip_important(v: &str) -> &str {
    let t = v.trim();
    match t.len().checked_sub(10) {
        Some(at) if t.is_char_boundary(at) && t[at..].eq_ignore_ascii_case("!important") => t[..at].trim_end(),
        _ => t,
    }
}

/// True when a value is one colour (with or without `!important`).
fn is_one_color(v: &str) -> bool {
    color::parse(strip_important(v)).is_some()
}

/// A length: `12`, `12px` or `60%` (anything else is no length to go by).
fn parse_dim(v: &str) -> Option<Dim> {
    let v = v.trim();
    let (num, pct) = match v.strip_suffix('%') {
        Some(n) => (n, true),
        None => (v.strip_suffix("px").or_else(|| v.strip_suffix("PX")).unwrap_or(v), false),
    };
    let n: f64 = num.trim().parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    Some(if pct { Dim::Pct(n) } else { Dim::Px(n) })
}

/// `width` and `height` of a declaration list.
fn dims_of(css: &str) -> (Option<Dim>, Option<Dim>) {
    let (mut w, mut h) = (None, None);
    for d in split_declarations(css) {
        match prop_of(&css[d.name.clone()]).1.as_ref() {
            "width" => w = parse_dim(strip_important(&css[d.value.clone()])),
            "height" => h = parse_dim(strip_important(&css[d.value.clone()])),
            _ => {}
        }
    }
    (w, h)
}

/// An empty painted element sized like a mark rather than a box: a rule
/// (4px or thinner), a dot, swatch or small bar (both sides given, 64px or
/// less), or a progress fill (16px or less tall, a share of the width).
/// A spacer row of a coloured band (only a height, over 4px) is not one.
fn is_mark(width: Option<Dim>, height: Option<Dim>, empty: bool) -> bool {
    let px = |d: Option<Dim>, max: f64| matches!(d, Some(Dim::Px(v)) if v <= max);
    empty && (px(height, 4.0) || px(width, 4.0) || px(width, 64.0) && px(height, 64.0) || px(height, 16.0) && matches!(width, Some(Dim::Pct(p)) if p < 100.0))
}

/// An empty painted element as narrow as a chart bar (64px or less) with
/// a height given: a vertical bar, however tall.
fn is_bar(width: Option<Dim>, height: Option<Dim>, empty: bool) -> bool {
    empty && matches!(width, Some(Dim::Px(w)) if w <= 64.0) && matches!(height, Some(Dim::Px(_)))
}

/// An image that only holds space: 2px or less on a side, or a remote file
/// named like a spacer (`spacer.gif`, `blank.png`, `1x1.gif`).
fn is_spacer(src: &str, tag: &Tag) -> bool {
    let attr = |n: &str| tag.attrs.iter().find(|a| src[a.name.clone()].eq_ignore_ascii_case(n)).map(|a| decode_attr(&src[a.value.clone()]).trim().to_ascii_lowercase());
    if ["width", "height"].iter().any(|n| attr(n).and_then(|v| parse_dim(&v)).is_some_and(|d| matches!(d, Dim::Px(v) if v <= 2.0))) {
        return true;
    }
    let Some(url) = attr("src").filter(|u| !u.starts_with("data:")) else { return false };
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let base = path.rsplit('/').next().unwrap_or_default();
    let stem = base.split('.').next().unwrap_or_default();
    ["spacer", "blank", "clear", "pixel", "transparent", "trans", "1x1", "s", "spc", "shim", "space", "empty"].contains(&stem) || base.contains("spacer") || base.contains("1x1")
}

/// True when nothing visible follows a start tag before an end tag
/// (white space, `&nbsp;`, a `<br>` or a spacer image at most).
fn is_empty_element(src: &str, from: usize) -> bool {
    let b = src.as_bytes();
    let mut i = from;
    for _ in 0..4 {
        loop {
            if i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            } else if let Some(n) = ["&nbsp;", "&#160;", "&#xa0;", "\u{a0}", "&zwnj;", "\u{200c}"].iter().find(|n| src[i..].starts_with(*n)) {
                i += n.len();
            } else {
                break;
            }
        }
        if src[i..].starts_with("</") {
            return true;
        }
        if is_tag_at(src, "<br", i) {
            i = src[i..].find('>').map_or(b.len(), |e| i + e + 1);
            continue;
        }
        // An image is content (an icon on a tile), unless it is a spacer.
        if is_tag_at(src, "<img", i) {
            match parse_tag(src, i) {
                Some(t) if is_spacer(src, &t) => {
                    i = t.end;
                    continue;
                }
                _ => return false,
            }
        }
        return false;
    }
    false
}

/// True for a group rule that applies in the dark frame: `@media` for the
/// dark colour scheme (the frame's `prefers-color-scheme` is dark), whatever
/// else it asks for.
fn is_dark_media(prelude: &str) -> bool {
    let p = strip_comments(prelude).to_ascii_lowercase();
    let p: String = p.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    p.starts_with("@media") && p.contains("prefers-color-scheme:dark") && !p.contains("not")
}

/// `@media`, `@supports` and the like hold rules; `@font-face`, `@page`
/// hold declarations. CDO/CDC (`<!--`, `-->`) and comments before the
/// prelude are skipped.
fn is_group_rule(prelude: &str) -> bool {
    let mut p = prelude.trim_start();
    loop {
        if let Some(rest) = p.strip_prefix("<!--").or_else(|| p.strip_prefix("-->")) {
            p = rest.trim_start();
        } else if let Some(rest) = p.strip_prefix("/*") {
            p = rest.find("*/").map_or("", |e| &rest[e + 2..]).trim_start();
        } else {
            break;
        }
    }
    let Some(at) = p.strip_prefix('@') else { return false };
    let name: String = at.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>().to_ascii_lowercase();
    matches!(
        name.as_str(),
        "media" | "supports" | "document" | "-moz-document" | "layer" | "container" | "scope" | "starting-style" | "keyframes" | "-webkit-keyframes" | "-moz-keyframes" | "-o-keyframes"
    )
}

/// A newline to the CSS parser (which turns CR, CR LF and FF into LF).
fn is_newline(c: u8) -> bool {
    matches!(c, b'\n' | b'\r' | b'\x0c')
}

/// True when the backslash at `i` starts an escape (not before a newline
/// or at the end).
fn is_escape(b: &[u8], i: usize) -> bool {
    b[i] == b'\\' && b.get(i + 1).is_some_and(|&n| !is_newline(n))
}

/// Skip a string (from its opening quote); returns the index after its
/// closing quote, or of the newline that ends it unclosed. A backslash
/// before a newline (CR LF included) continues it; an escape (`\1` and the
/// white space after it) is skipped whole.
fn skip_string(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => {
                j = match b.get(j + 1) {
                    Some(b'\r') if b.get(j + 2) == Some(&b'\n') => j + 3,
                    Some(&n) if is_newline(n) => j + 2,
                    Some(_) => escape_end(b, j),
                    None => j + 1,
                }
            }
            c if c == q => return j + 1,
            c if is_newline(c) => return j,
            _ => j += 1,
        }
    }
    b.len()
}

/// From `i` (an identifier character or an escape), the end of the name
/// and whether it reads `url` (escapes decoded, any case).
fn ident_run(b: &[u8], i: usize) -> (usize, bool) {
    let mut j = i;
    let mut escaped = false;
    while j < b.len() {
        if is_ident_byte(b[j]) {
            j += 1;
        } else if is_escape(b, j) {
            escaped = true;
            j = escape_end(b, j);
        } else {
            break;
        }
    }
    let name = &b[i..j];
    let url = if escaped {
        name.len() <= 24 && std::str::from_utf8(name).is_ok_and(|n| css_unescape(n).eq_ignore_ascii_case("url"))
    } else {
        name.eq_ignore_ascii_case(b"url")
    };
    (j, url)
}

/// From just after `url(`: the index after an unquoted url token (its first
/// unescaped `)`, or the end, as a bad url runs), or `None` when a quote
/// follows (then it is an ordinary function holding a string).
fn url_token_end(b: &[u8], open: usize) -> Option<usize> {
    let mut k = open;
    while k < b.len() && b[k].is_ascii_whitespace() {
        k += 1;
    }
    if matches!(b.get(k), Some(b'"' | b'\'')) {
        return None;
    }
    while k < b.len() {
        match b[k] {
            b')' => return Some(k + 1),
            b'\\' if is_escape(b, k) => k += 2,
            _ => k += 1,
        }
    }
    Some(b.len())
}

/// From `i`, past one string, comment, name (an unquoted `url(...)`
/// included) or hash/at-name, as the CSS tokenizer reads them; `None` for
/// anything else (a single character). Used where only the structure
/// (`;`, `:`, braces and brackets) matters.
fn skip_token(b: &[u8], i: usize) -> Option<usize> {
    match b[i] {
        b'"' | b'\'' => Some(skip_string(b, i)),
        b'/' if b.get(i + 1) == Some(&b'*') => Some(skip_comment(b, i)),
        // `<!--` and `-->` are tokens of their own (`<!--url(` is a url).
        b'<' if b[i + 1..].starts_with(b"!--") => Some(i + 4),
        b'-' if b[i + 1..].starts_with(b"->") => Some(i + 3),
        b'#' | b'@' if b.get(i + 1).is_some_and(|&n| is_ident_byte(n)) || i + 1 < b.len() && is_escape(b, i + 1) => Some(ident_run(b, i + 1).0),
        c if is_ident_byte(c) || is_escape(b, i) => {
            let (end, url) = ident_run(b, i);
            if url && b.get(end) == Some(&b'(')
                && let Some(e) = url_token_end(b, end + 1)
            {
                return Some(e);
            }
            Some(end)
        }
        // A backslash before a newline is a character of its own.
        b'\\' => Some(i + 1),
        _ => None,
    }
}

fn skip_comment(b: &[u8], i: usize) -> usize {
    let mut j = i + 2;
    while j + 1 < b.len() {
        if b[j] == b'*' && b[j + 1] == b'/' {
            return j + 2;
        }
        j += 1;
    }
    b.len()
}

/// From `p`, the next `;`, `{` or `}` outside strings, comments, url
/// tokens and brackets: (its index, the byte), or (len, None).
fn scan_item(css: &str, p: usize) -> (usize, Option<u8>) {
    let b = css.as_bytes();
    let mut depth = 0usize;
    let mut i = p;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth = depth.saturating_sub(1),
            c @ (b';' | b'{' | b'}') if depth == 0 => return (i, Some(c)),
            // A brace always ends an item, even inside an unclosed bracket.
            c @ (b'{' | b'}') => return (i, Some(c)),
            _ => {
                if let Some(e) = skip_token(b, i) {
                    i = e;
                    continue;
                }
            }
        }
        i += 1;
    }
    (b.len(), None)
}

/// From just after a `{`, the index after its matching `}` (or the end).
fn skip_block(css: &str, start: usize) -> usize {
    let mut depth = 1usize;
    let mut p = start;
    loop {
        let (end, term) = scan_item(css, p);
        match term {
            Some(b'{') => depth += 1,
            Some(b'}') => {
                depth -= 1;
                if depth == 0 {
                    return end + 1;
                }
            }
            Some(_) => {}
            None => return css.len(),
        }
        p = end + 1;
    }
}

struct Decl {
    name: Range<usize>,
    value: Range<usize>,
}

/// `name: value` pairs of a declaration list, split at top-level `;`
/// (outside strings, comments, url tokens and any brackets, braces too).
fn split_declarations(css: &str) -> Vec<Decl> {
    let b = css.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut colon = None;
    let mut depth = 0usize;
    let mut i = 0;
    let mut push = |start: usize, end: usize, colon: Option<usize>| {
        if let Some(c) = colon {
            out.push(Decl { name: start..c, value: c + 1..end });
        }
    };
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 && colon.is_none() => colon = Some(i),
            b';' if depth == 0 => {
                push(start, i, colon);
                start = i + 1;
                colon = None;
            }
            _ => {
                if let Some(e) = skip_token(b, i) {
                    i = e;
                    continue;
                }
            }
        }
        i += 1;
    }
    push(start, b.len().max(start), colon);
    out
}

/// The declaration list with `add` appended (after a `;` when needed),
/// when the browser reads what was added as declarations of their own:
/// `None` when the list ends inside a string, comment, url, bracket or
/// escape (`\;`), where they would be swallowed.
fn append_decls(css: &str, add: &str) -> Option<String> {
    let t = css.trim_end_matches([' ', '\t', '\n', '\r', '\x0c']);
    let sep = if t.is_empty() || t.ends_with(';') { "" } else { ";" };
    let out = format!("{css}{sep}{add}");
    let (got, want) = (split_declarations(&out), split_declarations(add));
    let tail = got.len().checked_sub(want.len()).map(|n| &got[n..])?;
    tail.iter().zip(&want).all(|(g, w)| out[g.name.clone()] == add[w.name.clone()] && out[g.value.clone()] == add[w.value.clone()]).then_some(out)
}

/// The end of a number from `i` (a digit, or `.` and a digit): digits, a
/// fraction, an exponent.
fn number_end(b: &[u8], i: usize) -> usize {
    let digits = |mut j: usize| {
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        j
    };
    let mut j = digits(i);
    if b.get(j) == Some(&b'.') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
        j = digits(j + 1);
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        let k = j + 1 + usize::from(matches!(b.get(j + 1), Some(b'+' | b'-')));
        if b.get(k).is_some_and(u8::is_ascii_digit) {
            j = digits(k);
        }
    }
    j
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c >= 0x80
}

/// The index after a CSS escape at `i` (a backslash): up to six hex digits
/// and one whitespace (CR LF counts as one), or one character.
fn escape_end(b: &[u8], i: usize) -> usize {
    let mut j = i + 1;
    if j < b.len() && b[j].is_ascii_hexdigit() {
        while j < b.len() && j < i + 7 && b[j].is_ascii_hexdigit() {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_whitespace() {
            j += if b[j] == b'\r' && b.get(j + 1) == Some(&b'\n') { 2 } else { 1 };
        }
        j
    } else if j < b.len() {
        // The whole character (UTF-8 continuation bytes are 10xxxxxx).
        j += 1;
        while j < b.len() && b[j] & 0xc0 == 0x80 {
            j += 1;
        }
        j
    } else {
        j
    }
}

/// Walk a CSS value and offer every colour token (hex, colour function,
/// named colour, including inside gradients and other functions, and a
/// `var()` fallback) to `f`; a `Some` from `f` replaces the token. `url()`
/// and the like, strings, comments, escaped names and functions with
/// escaped names are never looked into. A replacement never runs into the
/// token next to it (`solid#0000001f` becomes `solid rgba(...)`). `None`
/// when nothing was replaced. Linear: brackets are matched once up front,
/// and nesting deeper than [`MAX_FN_DEPTH`] is copied as it is.
fn rewrite_colors(v: &str, f: &mut dyn FnMut(Rgba) -> Option<String>) -> Option<String> {
    let mut w = ColorWalk { v, out: String::new(), last: 0, pairs: None };
    w.walk(0, v.len(), 0, f);
    if w.last == 0 {
        return None;
    }
    w.out.push_str(&v[w.last..]);
    Some(w.out)
}

struct ColorWalk<'a> {
    v: &'a str,
    out: String,
    last: usize,
    /// `(` index → matching `)` index, built on first use.
    pairs: Option<HashMap<usize, usize>>,
}

impl ColorWalk<'_> {
    fn close(&mut self, open: usize) -> Option<usize> {
        let v = self.v;
        self.pairs.get_or_insert_with(|| paren_pairs(v)).get(&open).copied()
    }

    /// Replace `range` by `new`, with a space before it when it would
    /// otherwise continue the name, number or hash before it (`glued`:
    /// one in the value, `solid rgba(`, `\ rgba(`; or one just written,
    /// `@ rgba(`, `#ffffff rgba(`), and after it when a `#hex` would
    /// swallow the name after it.
    fn replace(&mut self, range: Range<usize>, new: String, glued: bool) {
        let b = self.v.as_bytes();
        self.out.push_str(&self.v[self.last..range.start]);
        let pre = self.out.as_bytes();
        if new.starts_with(|c: char| c.is_ascii_alphabetic()) && (glued || pre.last().is_some_and(|&c| is_ident_byte(c) || c == b'@' || c == b'#')) {
            // A hex escape right before (`\63`) takes one white space as
            // its end: then a second one separates.
            let hex = pre.iter().rev().take_while(|c| c.is_ascii_hexdigit()).count();
            let slashes = pre[..pre.len() - hex].iter().rev().take_while(|&&c| c == b'\\').count();
            self.out.push_str(if (1..=6).contains(&hex) && slashes % 2 == 1 { "  " } else { " " });
        }
        self.out.push_str(&new);
        if new.starts_with('#') && range.end < b.len() && (is_ident_byte(b[range.end]) || is_escape(b, range.end)) {
            self.out.push(' ');
        }
        self.last = range.end;
    }

    fn walk(&mut self, start: usize, end: usize, depth: usize, f: &mut dyn FnMut(Rgba) -> Option<String>) {
        let v = self.v;
        let b = v.as_bytes();
        // Where the last name, number or hash in the value ended (with the
        // white space an escape at its end takes): a token there is glued.
        let mut glue = usize::MAX;
        let mut i = start;
        while i < end {
            let c = b[i];
            match c {
                b'"' | b'\'' => i = skip_string(b, i),
                b'/' if b.get(i + 1) == Some(&b'*') => i = skip_comment(b, i),
                b'\\' if !is_escape(b, i) => i += 1,
                b'<' if b[i + 1..].starts_with(b"!--") => i += 4,
                b'-' if b[i + 1..].starts_with(b"->") => i += 3,
                // An at-keyword (`@tan`) is no colour name.
                b'@' if i + 1 < end && (is_ident_byte(b[i + 1]) || is_escape(b, i + 1)) => {
                    i = ident_run(b, i + 1).0.min(end);
                    glue = i;
                }
                b'#' => {
                    // A hash token: its name may hold escapes (`#fff\31` is
                    // `#fff1`), then it is copied as it is.
                    let (e, _) = ident_run(b, i + 1);
                    let e = e.min(end);
                    let escaped = b[i + 1..e].contains(&b'\\');
                    if !escaped && let Some(new) = color::parse(&v[i..e]).and_then(&mut *f) {
                        self.replace(i..e, new, i == glue);
                    }
                    i = e.max(i + 1);
                    glue = i;
                }
                b'.' if !b.get(i + 1).is_some_and(u8::is_ascii_digit) => i += 1,
                b'0'..=b'9' | b'.' => {
                    // A number with its unit (`12px`, `1.5em`, `50%`): a
                    // name may follow `%` (`100%url(` is a url).
                    i = number_end(b, i);
                    if i < end && b[i] == b'%' {
                        i += 1;
                    } else if i < end && (is_ident_byte(b[i]) || is_escape(b, i)) {
                        i = ident_run(b, i).0;
                    }
                    i = i.min(end);
                    glue = i;
                }
                _ if is_ident_byte(c) || c == b'\\' => {
                    let (e, url) = ident_run(b, i);
                    let escaped = b[i..e].contains(&b'\\');
                    let e = e.min(end);
                    if e < end && b[e] == b'(' {
                        // An unquoted url: copied up to its end.
                        if url && let Some(u) = url_token_end(b, e + 1) {
                            i = u.min(end);
                            continue;
                        }
                        let Some(close) = self.close(e).filter(|&c| c < end) else {
                            // Unclosed: leave the rest as it is.
                            return;
                        };
                        if !escaped {
                            let name = v[i..e].to_ascii_lowercase();
                            match name.as_str() {
                                "rgb" | "rgba" | "hsl" | "hsla" | "hwb" | "oklab" | "oklch" => {
                                    if let Some(new) = color::parse(&v[i..=close]).and_then(&mut *f) {
                                        self.replace(i..close + 1, new, false);
                                    }
                                }
                                // The fallback, after the first top-level comma.
                                "var" => {
                                    if depth < MAX_FN_DEPTH
                                        && let Some(comma) = self.top_comma(e + 1, close)
                                    {
                                        self.walk(comma + 1, close, depth + 1, f);
                                    }
                                }
                                "url" | "env" | "attr" | "image" | "image-set" | "-webkit-image-set" | "element" | "expression" | "src" => {}
                                _ => {
                                    if depth < MAX_FN_DEPTH {
                                        self.walk(e + 1, close, depth + 1, f);
                                    }
                                }
                            }
                        }
                        // An escaped function name (`u\72 l(`) may be any
                        // function, url() included: copied as it is.
                        i = close + 1;
                    } else {
                        if !escaped
                            && color::is_named(&v[i..e])
                            && let Some(new) = color::parse(&v[i..e]).and_then(&mut *f)
                        {
                            self.replace(i..e, new, false);
                        }
                        i = e.max(i + 1);
                        glue = i;
                    }
                }
                _ => i += 1,
            }
        }
    }

    /// The first comma in `start..end` outside nested brackets, strings,
    /// comments and url tokens.
    fn top_comma(&mut self, start: usize, end: usize) -> Option<usize> {
        let b = self.v.as_bytes();
        let mut i = start;
        while i < end {
            match b[i] {
                b'(' => i = self.close(i).map_or(end, |c| c + 1),
                b',' => return Some(i),
                _ => i = skip_token(b, i).unwrap_or(i + 1),
            }
        }
        None
    }
}

/// Every `(` in a value with its matching `)`, in one pass (strings,
/// comments, escapes and url tokens skipped the way the CSS tokenizer
/// reads them).
fn paren_pairs(v: &str) -> HashMap<usize, usize> {
    let b = v.as_bytes();
    let mut open = Vec::new();
    let mut pairs = HashMap::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' => open.push(i),
            b')' => {
                if let Some(o) = open.pop() {
                    pairs.insert(o, i);
                }
            }
            _ => {
                if let Some(e) = skip_token(b, i) {
                    i = e;
                    continue;
                }
            }
        }
        i += 1;
    }
    pairs
}

struct Attr {
    name: Range<usize>,
    /// The value without quotes.
    value: Range<usize>,
    /// The value with its quotes (what an edit replaces).
    token: Range<usize>,
}

struct Tag {
    name: Range<usize>,
    attrs: Vec<Attr>,
    /// Index after the `>`.
    end: usize,
    self_closing: bool,
}

/// Parse the start tag at `lt`; `None` when it never ends.
fn parse_tag(src: &str, lt: usize) -> Option<Tag> {
    let b = src.as_bytes();
    let ws = |c: u8| c.is_ascii_whitespace();
    let mut i = lt + 1;
    while i < b.len() && !ws(b[i]) && b[i] != b'>' && b[i] != b'/' {
        i += 1;
    }
    let name = lt + 1..i;
    let mut attrs = Vec::new();
    let mut slash = false;
    loop {
        while i < b.len() && (ws(b[i]) || b[i] == b'/') {
            slash = b[i] == b'/';
            i += 1;
        }
        let &c = b.get(i)?;
        if c == b'>' {
            return Some(Tag { name, attrs, end: i + 1, self_closing: slash });
        }
        slash = false;
        let ns = i;
        i += 1;
        while i < b.len() && !ws(b[i]) && !matches!(b[i], b'>' | b'/' | b'=') {
            i += 1;
        }
        let an = ns..i;
        let mut j = i;
        while j < b.len() && ws(b[j]) {
            j += 1;
        }
        if b.get(j) != Some(&b'=') {
            attrs.push(Attr { name: an, value: i..i, token: i..i });
            continue;
        }
        j += 1;
        while j < b.len() && ws(b[j]) {
            j += 1;
        }
        match b.get(j) {
            Some(&q @ (b'"' | b'\'')) => {
                let close = j + 1 + src[j + 1..].find(q as char)?;
                attrs.push(Attr { name: an, value: j + 1..close, token: j..close + 1 });
                i = close + 1;
            }
            _ => {
                let vs = j;
                while j < b.len() && !ws(b[j]) && b[j] != b'>' {
                    j += 1;
                }
                attrs.push(Attr { name: an, value: vs..j, token: vs..j });
                i = j;
            }
        }
    }
}

/// Decode the character references html5ever writes in attribute values
/// (and numeric ones); anything else is left as it is.
fn decode_attr(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        // Bytes, not a str slice: byte 12 may fall inside a character
        // ("&quot;, 微软雅黑"); `;` is ASCII, so its index is a boundary.
        let semi = rest.as_bytes().iter().take(12).position(|&b| b == b';');
        let decoded = semi.and_then(|e| {
            let ent = &rest[1..e];
            let ch = match ent {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "nbsp" => Some('\u{a0}'),
                _ => {
                    let num = ent.strip_prefix('#')?;
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok(),
                        None => num.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }
            };
            ch.map(|c| (c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Escape a value for a double-quoted attribute, as html5ever does.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\u{a0}' => out.push_str("&nbsp;"),
            _ => out.push(c),
        }
    }
    out
}

/// True when the tag `<name` (given with its `<`, any case) starts at
/// `at`: the name followed by white space, `/` or `>`.
fn is_tag_at(src: &str, open: &str, at: usize) -> bool {
    let b = src.as_bytes();
    b.get(at..at + open.len()).is_some_and(|t| t.eq_ignore_ascii_case(open.as_bytes())) && b.get(at + open.len()).is_some_and(|&c| c.is_ascii_whitespace() || c == b'/' || c == b'>')
}

/// The next tag `<name` (given with its `<`) from `from` ([`is_tag_at`]).
fn find_tag(src: &str, open: &str, from: usize) -> Option<usize> {
    let mut i = from;
    while let Some(at) = find_ci(src, open, i) {
        if is_tag_at(src, open, at) {
            return Some(at);
        }
        i = at + 1;
    }
    None
}

/// The end tag of a raw-text element (`</style`) from `from`, as the HTML
/// parser finds it: followed by white space, `/` or `>`. `</stylex` is
/// text of the element.
fn find_end_tag(src: &str, name: &str, from: usize) -> Option<usize> {
    find_tag(src, &format!("</{name}"), from)
}

/// ASCII case-insensitive search from `from`.
fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    (from..=h.len().checked_sub(n.len())?).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    find_ci(hay, needle, 0).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::sanitize_email_html;

    fn adapt(html: &str) -> String {
        adapt_for_dark(html, false)
    }

    /// The colour a CSS colour token in `s` after `prefix` maps to.
    fn after<'a>(s: &'a str, prefix: &str) -> &'a str {
        let i = s.find(prefix).unwrap_or_else(|| panic!("{prefix} not in {s}")) + prefix.len();
        let rest = &s[i..];
        let end = rest.find([';', '"', ' ', '}', '!']).unwrap_or(rest.len());
        &rest[..end]
    }

    fn lightness(css: &str) -> f64 {
        color::parse(css).unwrap_or_else(|| panic!("{css}")).lightness()
    }

    #[test]
    fn inline_text_and_backgrounds_are_adapted() {
        let out = adapt(r##"<div style="color:#333;background-color:#ffffff;font-family:Arial"><span style="color: #666">x</span></div>"##);
        let bg = color::parse(after(&out, "background-color:")).unwrap();
        assert!(bg.lightness() < 0.35, "{out}");
        let fg = color::parse(after(&out, "color:")).unwrap();
        let fg2 = color::parse(after(&out, "<span style=\"color: ")).unwrap();
        // Darker in, lighter out; both read on the mapped white.
        assert!(fg.lightness() > fg2.lightness() && fg2.lightness() > 0.7, "{out}");
        assert!(color::contrast(fg2, bg) >= 4.5, "{out}");
        assert!(out.contains(";font-family:Arial\""), "other declarations are untouched: {out}");
        // Light text and dark backgrounds are already fine.
        let dark = r##"<td style="background-color:#17171f;color:#ffffff"><a style="color:#7cf3ff">x</a></td>"##;
        assert_eq!(adapt(dark), dark);
    }

    #[test]
    fn attributes_are_adapted() {
        let out = adapt(r##"<table bgcolor="#FFFFFF"><tr><td bgcolor="whitesmoke"><font color="#888888" face="arial">x</font><font color="333333">y</font></td></tr></table>"##);
        assert!(!out.contains("#FFFFFF") && !out.contains("whitesmoke") && !out.contains("#888888") && !out.contains("\"333333\""), "{out}");
        assert!(lightness(after(&out, "<table bgcolor=\"")) < 0.35);
        assert!(lightness(after(&out, "<font color=\"")) > 0.7);
        assert!(out.contains("face=\"arial\""));
        // Text colours read on the background around them.
        let out = adapt(r##"<td bgcolor="#FFD814"><a style="color:#0F1111">Buy</a></td>"##);
        let bg = color::parse(after(&out, "bgcolor=\"")).unwrap();
        let fg = color::parse(after(&out, "color:")).unwrap();
        assert!(color::contrast(fg, bg) >= 4.5, "{out}");
    }

    #[test]
    fn style_blocks_are_adapted_selectors_untouched() {
        let css = "<style>\n.muted { color: #8a8a8a; font-size: 13px; }\n.btn{background-color:#0b5cff;color:#ffffff !important}\n@media only screen and (max-width: 620px) { .hero h1 { color: #222222 !important; } }\n@media (prefers-color-scheme: dark) { .sheet { background-color: #1a1d23 !important; color: #f2f4f7 !important } }\na:link, span.MsoHyperlink { color:#0563C1; }\n</style><p class=\"muted\">x</p>";
        let out = adapt(css);
        assert!(out.contains(".muted { color: #") && !out.contains("#8a8a8a"), "{out}");
        assert!(lightness(after(&out, ".muted { color: ")) > 0.75);
        // A mid-tone button darkens a little; its white text stays.
        assert!(out.contains("color:#ffffff !important}"), "{out}");
        // Inside @media, with !important kept.
        let h1 = after(&out, ".hero h1 { color: ");
        assert!(lightness(h1) > 0.8, "{out}");
        assert!(out.contains(&format!("color: {h1} !important; }} }}")), "{out}");
        // The email's own dark styles are already dark: untouched.
        assert!(out.contains("@media (prefers-color-scheme: dark) { .sheet { background-color: #1a1d23 !important; color: #f2f4f7 !important } }"), "{out}");
        assert!(out.contains("a:link, span.MsoHyperlink { color:#"), "{out}");
        assert!(out.ends_with("</style><p class=\"muted\">x</p>"));
        // Outlook's comment-wrapped sheet with @font-face and @page.
        let mso = "<style><!--\n@font-face\n\t{font-family:Calibri;\n\tpanose-1:2 15 5 2 2 2 4 3 2 4;}\np.MsoNormal\n\t{margin:0in;\n\tcolor:black;}\n@page WordSection1\n\t{size:8.5in 11.0in;}\n--></style>";
        let out = adapt(mso);
        assert!(out.contains("{font-family:Calibri;\n\tpanose-1:2 15 5 2 2 2 4 3 2 4;}"), "{out}");
        assert!(out.contains("\tcolor:#ffffff;}"), "{out}");
        assert!(out.contains("--></style>"));
    }

    #[test]
    fn rules_pair_text_with_their_own_background() {
        let out = adapt("<style>.tag { background: #e7f0ff; color: #1d4ed8; }</style>");
        let bg = color::parse(after(&out, "background: ")).unwrap();
        let fg = color::parse(after(&out, "color: ")).unwrap();
        assert!(color::contrast(fg, bg) >= 4.5, "{out}");
    }

    #[test]
    fn gradients_urls_and_important() {
        let out = adapt(r##"<div style="background: linear-gradient(180deg, #ffffff 0%, rgba(238,242,247,0.9) 100%) !important; color: #2d3748 !important">x</div>"##);
        assert!(out.contains("linear-gradient(180deg, #"), "{out}");
        assert!(!out.contains("#ffffff") && !out.contains("238,242,247"), "{out}");
        assert!(out.contains(" 0%, rgba(") && out.contains(", 0.9) 100%) !important;"), "{out}");
        assert!(out.contains("!important\""), "{out}");
        // The url is kept byte for byte (its #fragment is no colour); only
        // the colour token changes.
        let out = adapt(r##"<td style="background:#ffffff url(cid:paper@example#fff) no-repeat center / cover">x</td>"##);
        assert!(out.contains("url(cid:paper@example#fff) no-repeat center / cover"), "{out}");
        assert!(!out.starts_with("<td style=\"background:#ffffff"), "{out}");
        assert_eq!(out.matches("url(").count(), 1);
        let out = adapt(r##"<p style="background-image:url('a.png'), -webkit-gradient(linear, left top, left bottom, from(#fafafa), to(white))">x</p>"##);
        assert!(out.contains("url('a.png')") && !out.contains("#fafafa") && !out.contains("to(white)"), "{out}");
        // A var() keeps its name; its fallback colour is adapted.
        let out = adapt(r#"<p style="color: inherit; background-color: var(--bg, #fff)">x</p>"#);
        assert!(out.starts_with(r#"<p style="color: inherit; background-color: var(--bg, #"#) && !out.contains("#fff)"), "{out}");
        // Left alone: keywords, transparent, system colours, SVG paint, dark
        // shadows.
        for keep in [
            r#"<p style="color: currentColor; background: transparent">x</p>"#,
            r#"<p style="color: inherit; background-color: var(--bg)">x</p>"#,
            r#"<p style="color:windowtext">x</p>"#,
            r#"<p style="border-left: 2px solid; fill: #000; stroke: black; box-shadow: 0 0 2px #000">x</p>"#,
            r#"<p style="font-family: Tan, Navy; content: 'white'">x</p>"#,
        ] {
            assert_eq!(adapt(keep), keep);
        }
    }

    #[test]
    fn escaping_stays_inside_the_attribute() {
        // Quotes in the style (as html5ever writes them) survive the rewrite.
        let html = r#"<span style="font-family:&quot;Arial&quot;,sans-serif;color:#A6A6A6">x</span><p title="a > b &amp; c">y</p>"#;
        let out = adapt(html);
        assert!(out.starts_with(r#"<span style="font-family:&quot;Arial&quot;,sans-serif;color:#"#), "{out}");
        assert!(out.ends_with(r#"">x</span><p title="a > b &amp; c">y</p>"#), "{out}");
        // A declaration can't break out, even with hostile text in it.
        let out = adapt(r#"<p style="color:#000;content:&quot;&lt;/p&gt;&lt;script&gt;&quot;">x</p>"#);
        assert!(!out.contains("<script") && !out.contains("</p><"), "{out}");
        assert!(out.contains("&lt;/p&gt;&lt;script&gt;"), "{out}");
        // Text inside <style> stays inside it.
        let out = adapt("<style>p{color:#000}</style><p>&lt;style&gt;</p>");
        assert_eq!(out.matches("</style>").count(), 1);
        assert!(out.ends_with("</style><p>&lt;style&gt;</p>"));
    }

    #[test]
    fn output_is_what_the_sanitiser_would_write() {
        // Re-sanitising the adapted HTML changes nothing: no new tags,
        // attributes, urls or properties, and the same escaping.
        for raw in [FIXTURE_RECRUITER, FIXTURE_NEWSLETTER, FIXTURE_OUTLOOK, FIXTURE_GITHUB] {
            let clean = sanitize_email_html(raw, false).html;
            let out = adapt(&clean);
            assert_ne!(out, clean);
            assert_eq!(sanitize_email_html(&out, false).html, out);
            assert_eq!(out.matches("url(").count(), clean.matches("url(").count());
            assert_eq!(out.matches(':').count(), clean.matches(':').count());
            assert_eq!(out.matches('<').count(), clean.matches('<').count());
        }
    }

    #[test]
    fn adapting_twice_changes_nothing_more() {
        for raw in [FIXTURE_RECRUITER, FIXTURE_NEWSLETTER, FIXTURE_OUTLOOK, FIXTURE_GITHUB] {
            let once = adapt(&sanitize_email_html(raw, false).html);
            assert_eq!(adapt(&once), once);
        }
    }

    #[test]
    fn malformed_input_is_copied_not_dropped() {
        for html in ["<p style=\"color:#000", "<div style='color:#000'>x", "a < b", "<!-- <p style=\"color:#000\"> -->", "<style>p{color:#000", "<p style=\"color:rgb(0,0,0\">x</p>", "<style>}}}{{{p{color:#000}</style>"] {
            let out = adapt(html);
            assert!(out.len() + 16 >= html.len(), "{html} -> {out}");
        }
        assert!(adapt("<div style='color:#000'>x").starts_with("<div style=\"color:#"));
        assert_eq!(adapt("<!-- <p style=\"color:#000\"> -->"), "<!-- <p style=\"color:#000\"> -->");
        // Deep nesting doesn't recurse without bound.
        let deep = format!("<style>{}p{{color:#000}}{}</style>", "@media screen{".repeat(5000), "}".repeat(5000));
        assert!(adapt(&deep).len() >= deep.len() - 8);
    }

    #[test]
    fn mangled_input_never_panics() {
        // Deterministic mutations of the fixtures: cut anywhere (also inside
        // multi-byte characters' neighbours), splice in syntax and non-ASCII.
        let pieces = ["<", ">", "\"", "'", "\\", "(", ")", "{", "}", ";", ":", "#", "/*", "*/", "<!--", "-->", "</style>", "<style>", "é", "€", "😀", "\\é", "&quot;", "&#x", "&quot;, 微软雅黑", "&amp;éééééé", "rgb(", "url(", "url(data:x)", "var(--a,", "--a:", "style=\"", "bgcolor=", " color=#", "@media{", "@media (prefers-color-scheme: dark){", "!important", "<img src=\"data:x\">", "<a href=x>", " class=\"k\"", "[x]", "\\63 "];
        let fixtures = [FIXTURE_RECRUITER, FIXTURE_NEWSLETTER, FIXTURE_OUTLOOK, FIXTURE_GITHUB];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..600 {
            let mut s = fixtures[round % fixtures.len()].replace("color", "colour é color");
            for _ in 0..1 + next(6) {
                let mut at = next(s.len() + 1);
                while !s.is_char_boundary(at) {
                    at -= 1;
                }
                if next(3) == 0 {
                    let mut end = (at + next(40)).min(s.len());
                    while !s.is_char_boundary(end) {
                        end -= 1;
                    }
                    s.replace_range(at..end, "");
                } else {
                    s.insert_str(at, pieces[next(pieces.len())]);
                }
            }
            let out = adapt(&s);
            assert_eq!(adapt(&s), out);
        }
    }

    #[test]
    fn a_large_newsletter_is_quick() {
        let one: String = [FIXTURE_RECRUITER, FIXTURE_NEWSLETTER, FIXTURE_OUTLOOK, FIXTURE_GITHUB].iter().map(|f| sanitize_email_html(f, true).html).collect();
        let body = one.repeat(300_000 / one.len() + 1);
        let t = std::time::Instant::now();
        let out = adapt(&body);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        eprintln!("adapt_for_dark: {} bytes in {ms:.2} ms", body.len());
        assert!(out.len() >= body.len() - body.len() / 20);
        // Linear: twice the input, about twice the time (generous bound).
        let double = body.repeat(2);
        let t = std::time::Instant::now();
        adapt(&double);
        let ms2 = t.elapsed().as_secs_f64() * 1000.0;
        assert!(ms2 < ms * 4.0 + 5.0, "{ms} ms then {ms2} ms");
        // Generous for debug builds; release takes a few ms.
        assert!(ms < 2000.0, "{ms} ms for {} bytes", body.len());
    }

    /// The text colour and background colour a test reads off the output.
    fn pair(out: &str, fg_prefix: &str, bg_prefix: &str) -> f64 {
        let fg = color::parse(after(out, fg_prefix)).unwrap_or_else(|| panic!("fg in {out}"));
        let bg = color::parse(after(out, bg_prefix)).unwrap_or_else(|| panic!("bg in {out}"));
        color::contrast(fg, bg)
    }

    /// A small transparent PNG (as data: URI).
    const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGNgYGD4DwABBAEAwS2OUAAAAABJRU5ErkJggg==";

    #[test]
    fn cjk_font_stacks_never_panic() {
        // The sanitiser writes the quotes as &quot;, so a multi-byte
        // character can sit at byte 12 after an entity.
        for raw in [
            r###"<p style='font-family:"Malgun Gothic", 맑은고딕, dotum, sans-serif; color:#333333'>안녕하세요</p>"###,
            r###"<table><tr><td style='font-family:"Microsoft YaHei", 微软雅黑, Arial; color:#666'>您好</td></tr></table>"###,
            r###"<p style='font-family:"ＭＳ Ｐゴシック", "Hiragino Kaku Gothic Pro"; color:#222'>こんにちは</p>"###,
        ] {
            let clean = sanitize_email_html(raw, false).html;
            assert!(clean.contains("&quot;"), "{clean}");
            let out = adapt(&clean);
            assert!(!out.contains("color:#333333") && !out.contains("color:#666;") && !out.contains("color:#222'"), "{out}");
            assert_eq!(sanitize_email_html(&out, false).html, out);
        }
        // Every multi-byte character at every offset after every entity.
        for entity in ["&quot;", "&amp;", "&nbsp;", "&#34;", "&lt;"] {
            for ch in ["é", "微", "😀"] {
                for at in 0..14 {
                    let s = format!("{entity}{}{}x;y", "a".repeat(at), ch.repeat(4));
                    let d = decode_attr(&s);
                    assert!(d.ends_with(&format!("{}x;y", ch.repeat(4))), "{s:?} -> {d:?}");
                    let s = format!("{}{entity}{ch}{ch}", "b".repeat(at));
                    let _ = decode_attr(&s);
                    let s = format!("&{}{ch};", "q".repeat(at));
                    assert_eq!(decode_attr(&s), s);
                }
            }
        }
    }

    #[test]
    fn deeply_nested_functions_are_bounded() {
        // 200k levels of an unknown function around a colour: no stack
        // overflow on a main-thread-sized stack, and linear time.
        let nested = |n: usize| format!("<p style=\"color:{}#333{}\">x</p>", "a(".repeat(n), ")".repeat(n));
        let big = nested(200_000);
        let out = std::thread::Builder::new().stack_size(8 << 20).spawn(move || adapt(&big)).unwrap().join().expect("no overflow");
        // Past the depth limit the value is copied as it is.
        assert!(out == nested(200_000), "{}", &out[..60]);
        let time = |n: usize| {
            let s = nested(n);
            let t = std::time::Instant::now();
            adapt(&s);
            t.elapsed().as_secs_f64()
        };
        let (t1, t2) = (time(20_000), time(80_000));
        assert!(t2 < t1 * 12.0 + 0.02, "{t1} s then {t2} s: not linear");
        // Shallow nesting is still looked into.
        let out = adapt(r###"<p style="background:-webkit-gradient(linear, left top, left bottom, from(rgba(255,255,255,1)), to(#fafafa))">x</p>"###);
        assert!(!out.contains("255,255,255") && !out.contains("#fafafa"), "{out}");
    }

    #[test]
    fn comments_and_escapes_before_property_names() {
        let sheet = "<style>body,#bodyTable{/*@editable*/background-color:#FAFAFA;}\n.mcnTextContent{\n\t/*@editable*/color:#202020;\n\t/*@editable*/font-size:16px;}\n.card{color:#333333;/* surface */background-color:#ffffff;}</style>";
        let out = adapt(&sanitize_email_html(sheet, false).html);
        assert!(!out.contains("#FAFAFA") && !out.contains("#202020") && !out.contains("background-color:#ffffff"), "{out}");
        assert!(out.contains("/*@editable*/background-color:#") && out.contains("/* surface */background-color:#"), "{out}");
        assert!(pair(&out, ".card{color:", "/* surface */background-color:") >= 4.5, "{out}");
        // Inline, with a comment first or between name and colon.
        for html in [r###"<div style="/* hdr */background:#ffffff;color:#222">x</div>"###, r###"<div style="background/* a */:#ffffff;color /* b */:#222">x</div>"###] {
            let out = adapt(html);
            assert!(!out.contains("#ffffff") && !out.contains("#222\""), "{out}");
        }
        let out = adapt(r###"<div style="/* hdr */background:#ffffff;color:#222">x</div>"###);
        assert!(pair(&out, "*/background:", "color:") >= 4.5, "{out}");
        // An escaped property name is the property the browser reads.
        let out = adapt(r###"<p style="\63 olor:#000;background:#fff">x</p>"###);
        assert!(out.starts_with(r###"<p style="\63 olor:#"###) && !out.contains("#000"), "{out}");
    }

    #[test]
    fn legacy_bgcolor_values_are_adapted_as_rendered() {
        // #fffff renders near white, #FFFFFF; renders yellow: both are light
        // boxes, darkened and written back as #rrggbb, text reading on them.
        for v in ["#fffff", "#FFFFFF;", "fff fff", "#ffffff80"] {
            let out = adapt(&format!(r###"<table bgcolor="{v}"><tr><td style="color:#333333">x</td></tr></table>"###));
            let bg = after(&out, "bgcolor=\"");
            let rendered = color::parse_legacy(v).unwrap();
            assert_eq!(bg.len(), 7, "{out}");
            assert_eq!(color::parse(bg), Some(color::map_background(rendered)).map(|c| color::parse(&c.to_css()).unwrap()), "{out}");
            assert!(pair(&out, "color:", "bgcolor=\"") >= 4.5, "{v}: {out}");
        }
        // bgcolor="fff" renders #0f0f0f and "white;" #0000e0: already dark,
        // left alone, and the text reads on what the browser paints.
        assert_eq!(adapt(r###"<td bgcolor="fff">x</td>"###), r###"<td bgcolor="fff">x</td>"###);
        let out = adapt(r###"<table bgcolor="white;"><tr><td style="color:#333333">x</td></tr></table>"###);
        assert!(out.starts_with(r###"<table bgcolor="white;">"###), "{out}");
        assert!(color::contrast(color::parse(after(&out, "color:")).unwrap(), color::parse_legacy("white;").unwrap()) >= 4.5, "{out}");
    }

    #[test]
    fn transparent_bgcolor_is_no_backdrop() {
        // The browser ignores bgcolor="transparent": the text sits on the
        // (mapped) white table and is lifted exactly as without the cell.
        let with = adapt(r##"<table bgcolor="#ffffff"><tr><td bgcolor="transparent"><span style="color:#ff0000">SALE</span></td></tr></table>"##);
        let without = adapt(r##"<table bgcolor="#ffffff"><tr><td><span style="color:#ff0000">SALE</span></td></tr></table>"##);
        assert!(with.contains("bgcolor=\"transparent\""), "{with}");
        assert_eq!(after(&with, "color:"), after(&without, "color:"));
        assert!(pair(&with, "color:", "<table bgcolor=\"") >= 4.5, "{with}");
    }

    #[test]
    fn random_documents_adapt_to_a_fixed_point() {
        let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..4000 {
            let (a, b, c, d) = (next() & 0xff_ffff, next() & 0xff_ffff, next() & 0xff, next() % 10);
            let html = format!(
                r##"<table bgcolor="#{a:06x}"><tr><td style="color:#{b:06x};border:1px solid #{c:02x}{c:02x}{c:02x};box-shadow:0 1px 2px #{a:06x}">x<a href="https://x.example">l</a><img src="{PNG}" width="20" height="20"></td><td style="background:rgba({},{},{},0.{d});color:#{b:06x}"><span style="color:#{a:06x}">y</span></td></tr></table><style>.k{{background:#{a:06x};color:#{b:06x};--c:#{b:06x};border-top:2px solid #{c:02x}0000}}.k a{{color:var(--c)}}</style><div class="k"><p>z</p></div>"##,
                a >> 16,
                (a >> 8) & 255,
                a & 255
            );
            let once = adapt(&html);
            assert_eq!(adapt(&once), once, "\n in:   {html}\n once: {once}");
        }
    }

    #[test]
    fn escaped_function_names_are_copied() {
        // u\72 l( is url( to the browser: nothing inside is touched.
        let out = adapt(r###"<p style="color:#000;background:u\72 l(https://red.example/white#fff.png) #fff">a</p>"###);
        assert!(out.contains(r"u\72 l(https://red.example/white#fff.png) #"), "{out}");
        assert!(!out.contains(".png) #fff\""), "{out}");
        assert_eq!(out.matches("url(").count(), 0);
    }

    #[test]
    fn background_pictures_keep_their_colours() {
        let pic = format!(r###"<td style="background-color:#f5efe6;background-image:url({PNG});background-size:cover"><h1 style="color:#2b2118">Winter Blend</h1><table><tr><td bgcolor="#ffffff" style="color:#333333">card</td></tr></table></td>"###);
        let out = adapt(&pic);
        // The picture, and the text over it, are as designed...
        assert!(out.starts_with(r###"<td style="background-color:#f5efe6;background-image:url("###) && out.contains(r###"<h1 style="color:#2b2118">"###), "{out}");
        // ...a box with its own background over it is adapted again.
        assert!(!out.contains(r###"bgcolor="#ffffff""###) && !out.contains("color:#333333"), "{out}");
        // Only a picture that will load counts: remote ones when allowed.
        let remote = r###"<td style="background:#f5efe6 url('https://cdn.example/paper.jpg')"><h1 style="color:#2b2118">x</h1></td>"###;
        assert_ne!(adapt_for_dark(remote, false), remote);
        assert_eq!(adapt_for_dark(remote, true), remote);
        // cid: pictures don't load in the reading pane.
        let cid = r###"<td style="background:#f5efe6 url(cid:paper)"><h1 style="color:#2b2118">x</h1></td>"###;
        assert_ne!(adapt(cid), cid);
        // A picture with no colour token at all: it was designed on the
        // white page, which it gets under it (see round 3's test).
        let only = format!(r###"<td style="background-image:url({PNG})"><h2 style="color:#2b2118">x</h2></td>"###);
        assert_eq!(adapt(&only), only.replace(")\">", ");background-color:#ffffff;color:#242424\">"));
        // Class based: the picture's rules and rules for classes used only
        // over it keep their colours; other rules are adapted.
        let sheet = format!(
            r###"<style>.hero{{background:#f5efe6 url({PNG}) no-repeat}}.hero h1{{color:#2b2118}}.kicker{{color:#8c6d46}}.body{{color:#333333}}</style><table><tr><td class="hero"><h1>Coffee</h1><p class="kicker">LIMITED</p></td></tr><tr><td class="body">Plain text</td></tr></table>"###
        );
        let out = adapt(&sheet);
        assert!(out.contains(".hero{background:#f5efe6 url(") && out.contains(".hero h1{color:#2b2118}") && out.contains(".kicker{color:#8c6d46}"), "{out}");
        assert!(!out.contains(".body{color:#333333}"), "{out}");
    }

    #[test]
    fn images_on_darkened_light_boxes_get_their_light_backing() {
        let out = adapt(&format!(r###"<table><tr><td bgcolor="#ffffff"><img src="{PNG}" width="120" height="29" alt="Logo"></td><td style="background-color:#fafafa"><a href="https://x.example"><img src="{PNG}" alt="Mark"></a> text</td></tr></table>"###));
        assert!(out.contains(r###"<img style="background-color:#ffffff;color:#242424" src="data:"###), "{out}");
        assert!(out.contains(r###"<img style="background-color:#fafafa;color:#115ea3" src="data:"###), "{out}");
        assert!(!out.contains(r###"bgcolor="#ffffff""###), "the cell itself is darkened: {out}");
        // Not for spacers, pictures that won't load, images with their own
        // backing (kept as designed), or images on dark backgrounds.
        for keep in [
            format!(r###"<td bgcolor="#ffffff"><img src="{PNG}" width="1" height="20"></td>"###),
            r###"<td bgcolor="#ffffff"><img src="cid:logo" width="120"></td>"###.to_owned(),
            r###"<td bgcolor="#ffffff"><img width="120" alt="blocked"></td>"###.to_owned(),
            format!(r###"<td bgcolor="#202124"><img src="{PNG}" width="120"></td>"###),
        ] {
            assert!(!adapt(&keep).contains("<img style"), "{keep}");
        }
        let own = format!(r###"<td bgcolor="#ffffff"><img src="{PNG}" style="background-color:#ffffff;border:0"></td>"###);
        assert!(adapt(&own).contains(r###"style="background-color:#ffffff;border:0""###));
        // Remote images when they load.
        let remote = r###"<td bgcolor="#ffffff"><img src="https://cdn.example/logo.png" width="120" alt="Logo"></td>"###;
        assert!(adapt_for_dark(remote, true).contains(r###"<img style="background-color:#ffffff;color:#242424" src="https:"###));
    }

    #[test]
    fn borders_follow_their_background() {
        // A white outline on a dark hero stays white (inline and class based).
        let out = adapt(r##"<td bgcolor="#111111"><table><tr><td style="border:2px solid #ffffff;padding:10px"><a style="color:#ffffff">Join</a></td></tr></table></td>"##);
        assert!(out.contains("border:2px solid #ffffff"), "{out}");
        let footer = "<style>.footer{background-color:#202124}.ghost{border:1px solid #ffffff;color:#ffffff}.rule{border-top:1px solid #ffffff}.card{border:1px solid #e6e6e6}</style><div class=\"footer\"><a class=\"ghost\" href=\"https://x.example\">Get the app</a><div class=\"rule\"></div></div><div class=\"card\">x</div>";
        let out = adapt(footer);
        assert!(out.contains(".ghost{border:1px solid #ffffff;") && out.contains(".rule{border-top:1px solid #ffffff}"), "{out}");
        // A light divider on a white card is darkened; a black grid line is
        // lifted so it stays visible on the canvas.
        assert!(!out.contains("#e6e6e6"), "{out}");
        let out = adapt(r##"<table style="border:1px solid #000000"><tr><td style="border-bottom:3px solid #222">x</td></tr></table>"##);
        assert!(color::contrast(color::parse(after(&out, "1px solid ")).unwrap(), color::DARK_CANVAS) >= 4.5, "{out}");
        assert!(color::contrast(color::parse(after(&out, "3px solid ")).unwrap(), color::DARK_CANVAS) >= 3.0, "{out}");
    }

    #[test]
    fn plain_links_on_coloured_cells_read() {
        for bg in ["#8bc34a", "#ff9800", "#ffc107", "#fff8dc", "#d4edda"] {
            let out = adapt(&format!(r###"<td style="background-color:{bg}"><a href="https://x.example">details</a> text</td>"###));
            assert!(out.contains(r###"<a style="color:#"###), "{out}");
            assert!(pair(&out, "<a style=\"color:", "background-color:") >= 4.5, "{bg}: {out}");
            assert_eq!(sanitize_email_html(&out, false).html, sanitize_email_html(&out, false).html);
        }
        let out = adapt(r###"<td bgcolor="#fff8dc"><a href="https://x.example" style="font-weight:bold">x</a></td>"###);
        assert!(out.contains(r###"style="font-weight:bold;color:#"###), "{out}");
        // Left to the frame: links that read, have a colour or a class, or
        // that the email's own rules colour.
        for keep in [
            r###"<td bgcolor="#202124"><a href="https://x.example">x</a></td>"###.to_owned(),
            r###"<p><a href="https://x.example">x</a></p>"###.to_owned(),
            r###"<td bgcolor="#8bc34a"><a class="btn" href="https://x.example">x</a></td>"###.to_owned(),
            r###"<td bgcolor="#8bc34a"><a href="https://x.example" style="color:#1a0dab">x</a></td>"###.to_owned(),
            r###"<style>a:link{color:#0563C1}</style><td bgcolor="#8bc34a"><a href="https://x.example">x</a></td>"###.to_owned(),
            r###"<style>.lnk a{color:#1a0dab}</style><td bgcolor="#8bc34a" class="lnk"><a href="https://x.example">x</a></td>"###.to_owned(),
        ] {
            assert!(!adapt(&keep).contains("<a style"), "{keep}");
        }
        // A rule scoped to another class leaves this link to the fix.
        let out = adapt(r###"<style>.lnk a{color:#1a0dab}</style><td bgcolor="#8bc34a"><a href="https://x.example">x</a></td>"###);
        assert!(out.contains("<a style=\"color:#"), "{out}");
    }

    #[test]
    fn custom_properties_are_adapted_by_use() {
        let out = adapt("<style>:root{--ink:#1f2328;--sub:#59636e;--card:#ffffff;--brand:#0969da}.ink{color:var(--ink)}.sub{color:var(--sub)}.card{background:var(--card)}.btn{background:var(--brand);border-color:var(--brand)}.lnk{color:var(--brand)}</style>");
        assert!(lightness(after(&out, "--ink:")) > 0.75 && lightness(after(&out, "--sub:")) > 0.7, "{out}");
        assert!(lightness(after(&out, "--card:")) < 0.35, "{out}");
        // Used for text and for a background: either mapping breaks one.
        assert!(out.contains("--brand:#0969da}"), "{out}");
        // Unused ones go by their name; non-colour values are left alone.
        let out = adapt(r###"<div style="--text-color:#222;--bg-color:#fff;--gap:4px;--shadow:0 1px #000">x</div>"###);
        assert!(lightness(after(&out, "--text-color:")) > 0.75 && lightness(after(&out, "--bg-color:")) < 0.35, "{out}");
        assert!(out.contains("--gap:4px;--shadow:0 1px #000"), "{out}");
    }

    #[test]
    fn hidden_text_stays_hidden() {
        let out = adapt(r##"<table bgcolor="#ffffff"><tr><td><span style="color:#ffffff;font-size:1px">preview text: 40% off</span><span style="color:#333">Hi</span></td></tr></table>"##);
        let bg = after(&out, "bgcolor=\"");
        assert_eq!(after(&out, "<span style=\"color:"), bg, "{out}");
        let hi = out.split("<span style=\"color:").nth(2).unwrap();
        let bg = color::parse(bg).unwrap();
        assert!(color::contrast(color::parse(&hi[..7]).unwrap(), bg) >= 4.5, "{out}");
        // White text on a background the email's own dark rules make dark
        // is meant to show.
        let swap = "<style>.bg{background-color:#ffffff}.logo-dark{display:none}@media (prefers-color-scheme: dark){.bg{background-color:#1c1c1e !important}.logo-dark{display:block !important}}</style><table class=\"bg\"><tr><td><div class=\"logo-dark\" style=\"color:#ffffff\">NORDICA</div></td></tr></table>";
        assert!(adapt(swap).contains("style=\"color:#ffffff\""), "{}", adapt(swap));
        // A background the adapter can't place (a rule for links in a
        // cell, two classes at once) may be what the white text is on.
        for sheet in ["<style>td.cta a{background:#0078d4}</style>", "<style>.btn.primary{background:#0078d4}</style>"] {
            let out = adapt(&format!(r###"{sheet}<table bgcolor="#ffffff"><tr><td class="cta"><a class="btn primary" href="https://x.example" style="color:#ffffff">Buy</a></td></tr></table>"###));
            assert!(out.contains("style=\"color:#ffffff\""), "{out}");
        }
        // A hover background is not one.
        let out = adapt(r###"<style>a:hover{background:#0078d4}</style><table bgcolor="#ffffff"><tr><td><span style="color:#ffffff">pre</span></td></tr></table>"###);
        assert!(!out.contains("color:#ffffff"), "{out}");
    }

    #[test]
    fn shadows_are_adapted_but_paint_no_backdrop() {
        let out = adapt(r###"<td style="background-color:#ffffff;box-shadow:0 2px 5px rgba(213,217,217,.5)"><p style="color:#555555;text-shadow:0 1px 0 #ffffff">x</p></td>"###);
        assert!(!out.contains("213,217,217") && !out.contains("0 1px 0 #ffffff"), "{out}");
        assert!(out.contains("box-shadow:0 2px 5px rgba("), "{out}");
        // Dark shadows already suit the dark canvas.
        let dark = r###"<p style="color:#ffffff;text-shadow:1px 1px 2px #000000;box-shadow:0 0 4px rgba(0,0,0,0.5)">x</p>"###;
        assert_eq!(adapt(dark), dark);
        // A light shadow is not the text's background.
        let out = adapt(r###"<div style="box-shadow:0 0 0 999px #ffffff;color:#777777">x</div>"###);
        assert!(color::contrast(color::parse(after(&out, "color:")).unwrap(), color::default_text_backdrop()) >= 4.5, "{out}");
    }

    #[test]
    fn additions_are_what_the_sanitiser_would_write() {
        let raw = format!(r###"<table bgcolor="#ffffff"><tr><td style="background-color:#ffc107"><a href="https://x.example">verify</a> <img src="{PNG}" width="40" height="40" alt="ok"></td></tr></table>"###);
        let clean = sanitize_email_html(&raw, false).html;
        let out = adapt(&clean);
        assert!(out.contains("<a style=\"color:#") && out.contains("<img style=\"background-image:radial-gradient(circle closest-side,#ffc107 97%,transparent 100%);color:#242424\""), "{out}");
        assert_eq!(sanitize_email_html(&out, false).html, out);
        assert_eq!(adapt(&out), out);
    }

    /// Adapting is a fixed point and the output re-sanitises to itself.
    fn stable(html: &str) -> String {
        let out = adapt(html);
        assert_eq!(adapt(&out), out, "twice:\n in:  {html}\n out: {out}");
        out
    }

    // ---- round-2 review ----

    #[test]
    fn a_style_element_ends_only_at_a_real_end_tag() {
        // `</stylex` is style text to the HTML parser: the sheet goes on.
        let clean = sanitize_email_html("<style>p{color:#000}</stylex .a{color:#111111}\r\n.b{content:'<'}</style><p class=\"a\">x</p>", false).html;
        assert!(clean.contains("</stylex"), "{clean}");
        let out = stable(&clean);
        assert!(!out.contains("#111111") && out.contains("</stylex .a{color:#"), "{out}");
        assert_eq!(sanitize_email_html(&out, false).html, out);
        assert_eq!(out.matches("</style>").count(), 1);
        // Upper case and white space before `>` still end it.
        let out = adapt("<style>p{color:#000}</STYLE ><p style=\"color:#000\">x</p>");
        assert!(out.contains("</STYLE ><p style=\"color:#") && !out.contains("color:#000\""), "{out}");
    }

    #[test]
    fn css_strings_end_at_cr_and_form_feed() {
        // The browser reads CR and FF as newlines, which end a string: the
        // name after it is string text...
        for nl in ["\u{c}", "&#13;", "\r"] {
            let html = format!("<p style=\"color:&quot;{nl}&quot;black\">x</p>");
            assert_eq!(adapt(&html), html);
        }
        // ...and a declaration after a broken string applies, so it is adapted.
        let out = stable("<p style=\"content:&quot;a\u{c};color:#000000\">x</p>");
        assert!(!out.contains("#000000"), "{out}");
        // A backslash before CR LF continues the string.
        let html = "<style>.a{content:\"x\\\r\n#000\";}</style>";
        assert_eq!(adapt(html), html);
        // A backslash before a form feed is no escape of a colour's name.
        let out = adapt("<p style=\"background:\\\u{c}white\">x</p>");
        assert!(!out.contains("white"), "{out}");
    }

    #[test]
    fn unquoted_urls_are_copied_whole() {
        // Braces, quotes and comments inside an unquoted url are url text.
        let html = "<style>.a{background-image:url(data:image/svg+xml,%3Csvg%3E{fill:black}%3C/svg%3E)}.b{color:#000}.c{background:url(a\"b/*#fff*/) #fff}</style>";
        let out = stable(html);
        assert!(out.contains("url(data:image/svg+xml,%3Csvg%3E{fill:black}%3C/svg%3E)}.b{color:#"), "{out}");
        assert!(!out.contains(".b{color:#000}") && out.contains("url(a\"b/*#fff*/) #"), "{out}");
        // Inline too, and an unclosed one runs to the end.
        let html = "<p style=\"background:url(x/*;color:#000\">x</p>";
        assert_eq!(adapt(html), html);
        let out = adapt("<p style=\"background:url(&quot;a)#fff&quot;);color:#000\">x</p>");
        assert!(out.contains("url(&quot;a)#fff&quot;)") && !out.contains("color:#000"), "{out}");
    }

    #[test]
    fn added_declarations_never_land_in_an_open_construct() {
        // The fuzzer's reproducers: an unclosed string, a trailing
        // backslash, an open comment or url. Nothing is added there, and a
        // second pass adds nothing either.
        for style in ["&quot;", "\\", "é\u{200b}=/*", ";url(e.&lt;", "background-image: solid/**/url(% \u{c}", "x:(", "x:{"] {
            let html = format!("<a bgcolor=\"chuc\" style=\"{style}\" href=\"https://x.example\">x</a>");
            let out = stable(&html);
            assert!(!out.contains(";color:#") || out.matches("color:#").count() == 1, "{out}");
            let img = format!("<td bgcolor=\"#ffffff\"><img src=\"{PNG}\" style=\"{style}\"></td>");
            let out = stable(&img);
            assert!(!out.contains("background-color:#ffffff"), "{out}");
        }
        // A clean list still gets it.
        let out = stable("<td bgcolor=\"#fff8dc\"><a href=\"https://x.example\" style=\"font-weight:bold\">x</a></td>");
        assert!(out.contains("style=\"font-weight:bold;color:#"), "{out}");
    }

    #[test]
    fn translucent_colours_never_merge_with_the_token_before() {
        let out = stable(r###"<p style="border:1px solid#0000004d;box-shadow:0 1px 2px#ffffff80;text-decoration:-#C05b">x</p>"###);
        assert!(out.contains("solid rgba(") && out.contains("2px rgba(") && out.contains("- rgba("), "{out}");
        // A name or hash glued before a colour, and a function glued to the name after it.
        let out = stable(r###"<p style="color:darkred#Da5fC0A7;border:1px rgb(0,0,0)solid">x</p>"###);
        assert!(out.contains(" rgba(") && !out.contains("#ffffffrgba") && out.contains(" solid"), "{out}");
        assert!(!out.contains("darkred"), "{out}");
        // A replaced function right before a replaced translucent hash.
        let out = stable(r###"<p style="color:hwb(0 0% 100%)#cbb2">x</p>"###);
        assert!(out.contains("#ffffff rgba("), "{out}");
        // After an escape that took its white space (`\31 `, `\ `).
        let out = stable(r###"<p style="border-top:#c65b223\31 #ddAE;box-shadow:0 0 1px \ #EBc112D2">x</p>"###);
        assert!(out.contains("\\31  rgba(") && out.contains("\\  rgba("), "{out}");
    }

    #[test]
    fn escaped_hex_colours_are_copied() {
        // `#fff\31` is one hash token, `#fff1`, to the browser.
        for html in [r###"<p style="color:#fff\31">x</p>"###, r###"<p style="color:#000\30">x</p>"###, r###"<p style="background-color:#5178cc\31 ">x</p>"###] {
            assert_eq!(adapt(html), html);
        }
    }

    #[test]
    fn image_text_and_translucent_backdrops_are_fixed_points() {
        for html in [
            r###"<img color="ff" style="\62 ackground:#cEA">"###,
            r###"<i color="0" style="background:#eBc1">x</i>"###,
            r###"<img color="#3e4e4b" style="background::#eBF #1fc1">"###,
            r###"<small color="#9c2609" style="\62 ackground:#A20D 0 8px 5px">x</small>"###,
        ] {
            stable(html);
        }
        // An image's own background is kept, and its alt text with it.
        let html = r###"<img style="background:#ffffff;color:#333333" src="cid:x" alt="Logo">"###;
        assert_eq!(adapt(html), html);
    }

    #[test]
    fn nested_rules_under_a_long_selector_are_linear() {
        let sheet = |n: usize| format!("<style>{}{{color:#000;{}}}</style>", (0..2000).map(|i| format!(".c{i}")).collect::<Vec<_>>().join(","), "a{color:#111}".repeat(n));
        let time = |n: usize| {
            let s = sheet(n);
            let t = std::time::Instant::now();
            adapt(&s);
            t.elapsed().as_secs_f64()
        };
        let (t1, t2) = (time(500), time(4000));
        assert!(t2 < t1 * 16.0 + 0.05, "{t1} s then {t2} s: not linear");
    }

    #[test]
    fn long_selectors_and_values_are_linear() {
        // White space runs in a prelude, and long values with many tokens.
        let make = |n: usize| format!("<style>.a{}b, .c{{color:#000}}</style><p style=\"box-shadow:{}\">x</p>", " ".repeat(n), "0 0 1px #fff, ".repeat(n / 16));
        let time = |n: usize| {
            let s = make(n);
            let t = std::time::Instant::now();
            adapt(&s);
            t.elapsed().as_secs_f64()
        };
        let (t1, t2) = (time(20_000), time(160_000));
        assert!(t2 < t1 * 16.0 + 0.05, "{t1} s then {t2} s: not linear");
    }

    #[test]
    fn images_with_no_known_background_get_a_plate() {
        // On the white page (no background anywhere above), a transparent
        // dark logo gets a white plate; icons a round one.
        let out = stable(&format!(r###"<div><img src="{PNG}" width="32" height="32" alt="GitHub"></div><p><img src="{PNG}" width="130" height="40" alt="amazon"> <img src="{PNG}" width="600" alt="hero"></p>"###));
        assert!(out.contains(r###"<img style="background-image:radial-gradient(circle closest-side,#ffffff 97%,transparent 100%);color:#242424" src="data:"###), "{out}");
        assert_eq!(out.matches(r###"<img style="background-color:#ffffff;color:#242424" src="data:"###).count(), 2, "{out}");
        // Not where a rule the adapter can't place may paint it dark.
        let out = adapt(&format!(r###"<style>td.hero img{{background:#000}} div.x{{background:#111}}</style><div class="x"><img src="{PNG}" width="32" height="32"></div>"###));
        assert!(!out.contains("<img style"), "{out}");
        // Spacers and pixels still get nothing.
        assert!(!adapt(&format!(r###"<p><img src="{PNG}" width="1" height="1"></p>"###)).contains("<img style"));
    }

    #[test]
    fn plates_keep_an_images_own_shape() {
        // A round plate for icons is painted, not clipped (a square icon
        // keeps its corners); other plates are the image's own box (no
        // ring past it: sliced images and full-width banners stay as they
        // are); an image with its own corners gets a plain one.
        let out = stable(&format!(r###"<table><tbody><tr><td bgcolor="#f2f2f2"><img src="{PNG}" width="28" height="28" alt="a"><img src="{PNG}" style="width:120px;height:30px" alt="b"><img src="{PNG}" width="40" height="40" style="border-radius:8px" alt="c"><img src="{PNG}" height="90" style="width:100%" alt="d"></td></tr></tbody></table>"###));
        assert!(out.contains(r###"<img style="background-image:radial-gradient(circle closest-side,#f2f2f2 97%,transparent 100%);color:#242424" src"###), "{out}");
        assert!(out.contains(r###"style="width:120px;height:30px;background-color:#f2f2f2;color:#242424""###), "{out}");
        assert!(out.contains(r###"style="border-radius:8px;background-color:#f2f2f2;color:#242424""###), "{out}");
        assert!(out.contains(r###"style="width:100%;background-color:#f2f2f2;color:#242424""###), "{out}");
        assert!(!out.contains("border-radius:50%") && !out.contains("box-shadow") && !out.contains("border-radius:4px"), "{out}");
        assert_eq!(sanitize_email_html(&out, false).html, out);
    }

    #[test]
    fn shared_class_text_over_a_picture_keeps_its_colour() {
        // .headline and .cta are used over the light hero and in the plain
        // body: their rules are adapted for the body, the hero keeps them.
        let html = format!(
            r###"<style>.headline{{color:#1d1d1f;font-size:26px}}.cta{{color:#0b5cad !important}}.kicker{{color:#00a0df}}.solo{{color:#333333}}</style><table><tr><td style="background-image:url({PNG})" bgcolor="#d6e8f6"><p class="headline">Fall into savings</p><a class="cta" href="https://x.example">Shop</a><p class="kicker" style="color:#00a0df">K</p><p class="solo">S</p></td></tr><tr><td><p class="headline">More</p><a class="cta" href="https://x.example">See</a><p class="kicker">K</p></td></tr></table>"###
        );
        let out = stable(&html);
        assert!(!out.contains(".headline{color:#1d1d1f") && !out.contains(".cta{color:#0b5cad"), "{out}");
        assert!(out.contains(r###"<p style="color:#1d1d1f" class="headline">Fall"###), "{out}");
        assert!(out.contains(r###"<a style="color:#0b5cad !important" class="cta""###), "{out}");
        // Its own colour already wins; a class only used over the picture
        // keeps its rule.
        assert!(out.contains(r###"<p class="kicker" style="color:#00a0df">K</p><p class="solo">S</p>"###) && out.contains(".solo{color:#333333}"), "{out}");
        // Rules that also colour the class elsewhere (`.hero .headline`) are
        // not second-guessed.
        let out = adapt(&html.replace("<style>", "<style>.x .headline{color:#ffffff}"));
        assert!(out.contains(r###"<p class="headline">Fall"###), "{out}");
    }

    #[test]
    fn dark_marks_on_light_backgrounds_stay_visible() {
        let card = |inner: &str| format!(r###"<table width="600" bgcolor="#ffffff"><tr><td>{inner}</td></tr></table>"###);
        let card_bg = color::map_background(color::WHITE);
        // A 2px rule painted as a cell, a 1px black div, a progress fill and
        // small bars: folded into the border band, visible on the dark card.
        let out = stable(&card(r###"<table width="100%"><tr><td height="2" bgcolor="#333333" style="font-size:1px;line-height:1px">&nbsp;</td></tr></table><div style="height:1px;background-color:#000000;font-size:1px">&nbsp;</div><div style="background-color:#e3e6e6;height:10px"><div style="background-color:#232f3e;width:60%;height:10px"></div></div><table><tr><td width="30" height="60" bgcolor="#1f3864"></td><td width="14" height="14" bgcolor="#444444"></td></tr></table>"###));
        for (prefix, min) in [("height=\"2\" bgcolor=\"", 3.0), ("height:1px;background-color:", 3.0), ("width=\"30\" height=\"60\" bgcolor=\"", 3.0), ("width=\"14\" height=\"14\" bgcolor=\"", 3.0)] {
            let c = color::parse(after(&out, prefix)).unwrap();
            assert!(color::contrast(c, card_bg) >= min, "{prefix}: {out}");
        }
        let fills: Vec<Rgba> = out.split("<div style=\"background-color:").skip(1).map(|s| color::parse(&s[..7]).unwrap()).collect();
        let (track, fill) = (fills[0], fills[1]);
        // On its darkened track (light 10.8:1), clearly apart.
        assert!(color::contrast(fill, track) >= 2.5, "{out}");
        // Not a spacer row of a coloured band (a height only), a mark with
        // text in it, or a white spacer on the white card.
        for keep in [r###"<td bgcolor="#14213d" height="10" style="font-size:1px">&nbsp;</td>"###, r###"<td bgcolor="#14213d" width="10" height="10">New</td>"###] {
            let out = adapt(&card(keep));
            assert!(out.contains("bgcolor=\"#14213d\""), "{out}");
        }
        let out = adapt(&card(r###"<table><tr><td height="2" bgcolor="#ffffff"></td></tr></table>"###));
        assert_eq!(out.matches(&format!("bgcolor=\"{}\"", card_bg.to_hex())).count(), 2, "{out}");
        // On a dark design, marks stay as designed.
        let html = r###"<table bgcolor="#111111"><tr><td><div style="height:2px;background:#ffffff"></div></td></tr></table>"###;
        assert_eq!(adapt(html), html);
    }

    #[test]
    fn the_senders_dark_mode_rules_are_kept() {
        let html = format!(
            r###"<style>.page{{background-color:#ffffff}}.logo-wrap{{background-color:transparent}}.ink{{color:#222222}}@media (prefers-color-scheme: dark){{ .page{{background-color:#1e1e1e!important}} .ink{{color:#eeeeee!important}} .logo-wrap{{background-color:#ffffff!important;border-radius:6px}} .accent{{background-color:#ffd814!important;color:#0f1111!important}} }}</style><table class="page"><tr><td class="logo-wrap"><img src="{PNG}" width="130" height="40" alt="Logo"><span style="color:#333333">Hi</span></td></tr><tr><td><img src="{PNG}" width="28" height="28" alt="Icon"></td></tr></table>"###
        );
        let out = stable(&html);
        assert!(out.contains("@media (prefers-color-scheme: dark){ .page{background-color:#1e1e1e!important} .ink{color:#eeeeee!important} .logo-wrap{background-color:#ffffff!important;border-radius:6px} .accent{background-color:#ffd814!important;color:#0f1111!important} }"), "{out}");
        assert!(!out.contains(".ink{color:#222222}"), "{out}");
        // On the sender's white plate: no plate of ours, text as designed.
        assert!(out.contains(r###"<td class="logo-wrap"><img src="###) && out.contains(r###"<span style="color:#333333">Hi"###), "{out}");
        // On the page the sender darkens, an image was designed on white.
        assert!(out.contains(r###"<img style="background-image:radial-gradient(circle closest-side,#ffffff 97%,transparent 100%);color:#242424" src"###), "{out}");
        // A logo the sender swaps for a white one in dark mode gets no plate.
        let swap = format!(r###"<style>.dark-logo{{display:none}}@media (prefers-color-scheme: dark){{.dark-logo{{display:block!important}}.light-logo{{display:none!important}}}}</style><div class="dark-logo"><img src="{PNG}" width="120" height="30"></div><img class="light-logo" src="{PNG}" width="120" height="30">"###);
        assert!(!stable(&swap).contains("<img style"), "{swap}");
    }

    #[test]
    fn vivid_fills_on_a_dark_design_stay() {
        let html = r###"<table bgcolor="#111111"><tr><td><table style="background-color:#ffe600;border:3px dashed #000000"><tr><td style="color:#000000">50% OFF <a href="https://x.example">code</a></td></tr></table><a href="https://x.example" style="background-color:#e4007c;color:#ffffff;box-shadow:0 0 14px #e4007c">FIND</a><td bgcolor="#ffe600"><span style="color:#000">x</span></td></td></tr></table>"###;
        let out = stable(html);
        assert!(out.contains(r###"<table style="background-color:#ffe600;border:3px dashed #000000;color:#242424"><tr><td style="color:#000000">50% OFF <a style="color:#115ea3" href"###), "{out}");
        assert!(out.contains(r###"style="background-color:#e4007c;color:#ffffff;box-shadow:0 0 14px #e4007c""###) && out.contains(r###"<td style="color:#242424" bgcolor="#ffe600"><span style="color:#000">"###), "{out}");
        // A class's dark text on the accent keeps its colour there.
        let out = stable(r###"<style>.ink{color:#000000}</style><p class="ink">plain</p><table bgcolor="#111111"><tr><td style="background-color:#ffe600"><span class="ink">50% OFF</span></td></tr></table>"###);
        assert!(!out.contains(".ink{color:#000000}") && out.contains(r###"<span style="color:#000000" class="ink">50% OFF"###), "{out}");
        // A white card on the dark design is content, not an accent; a
        // vivid fill on a light page is adapted as before.
        let out = adapt(r###"<table bgcolor="#111111"><tr><td style="background-color:#ffffff;color:#333333">card</td></tr></table><table bgcolor="#ffffff"><tr><td style="background-color:#ffe600">x</td></tr></table>"###);
        assert!(!out.contains("#ffffff") && !out.contains("#333333") && !out.contains("#ffe600"), "{out}");
    }

    #[test]
    fn rules_for_attributes_the_html_lacks_are_ignored() {
        // Outlook's own dark-mode rules never match in the frame: white text
        // on the white cell is still hidden text, links still get colours.
        let html = r###"<style>.btn{background-color:#1a73e8;color:#ffffff}[data-ogsb] .btn{background-color:#8ab4f8!important}[data-ogsc] a{color:#202124!important}</style><table bgcolor="#ffffff"><tr><td><span style="color:#ffffff;font-size:12px">Preview text</span></td></tr></table><table><tr><td bgcolor="#8bc34a"><a href="https://x.example">x</a></td></tr></table>"###;
        let out = stable(html);
        let bg = after(&out, "<table bgcolor=\"");
        assert_eq!(after(&out, "<span style=\"color:"), bg, "{out}");
        assert!(out.contains("<a style=\"color:#"), "{out}");
        // A rule for an attribute that is there may match: as before.
        let out = adapt(&html.replace("[data-ogsb]", "[title]").replace("<table bgcolor=\"#ffffff\">", "<table bgcolor=\"#ffffff\" title=\"t\">"));
        assert!(out.contains("<span style=\"color:#ffffff"), "{out}");
    }

    #[test]
    fn faded_light_text_stays_dimmer_than_ink() {
        // An empty rating star (#d2d2d2 on white) reads in dark mode but
        // doesn't outshine the filled ones.
        let out = stable(r###"<table bgcolor="#ffffff"><tr><td><span style="color:#f15c00">★★★★</span><span style="color:#d2d2d2">★</span><span style="color:#ffffff">hidden</span></td></tr></table>"###);
        let bg = color::parse(after(&out, "bgcolor=\"")).unwrap();
        let filled = color::parse(after(&out, "<span style=\"color:")).unwrap();
        let empty = color::parse(&out.split("<span style=\"color:").nth(2).unwrap()[..7]).unwrap();
        assert!(color::contrast(empty, bg) >= 4.5 && empty.lightness() < 0.8, "{out}");
        assert!(empty.luminance() < color::parse("#d2d2d2").unwrap().luminance(), "{out}");
        assert!(filled.luminance() > 0.0, "{out}");
        assert_eq!(&out.split("<span style=\"color:").nth(3).unwrap()[..7], bg.to_hex(), "{out}");
    }

    #[test]
    fn fuzz_reproducers_are_fixed_points() {
        // Minimised cases from the round-2 fixer's fuzz runs.
        for (html, remote) in [
            // An overflowing oklab() is no colour (it came out black, then white).
            ("<a style=\"--:oklab(7,1e308,-1e308)\">x</a>", false),
            // A mark over a picture, and on the unknown page.
            ("<s style=\"\\62 ackground/**/:#dDe02b URL(http:\"><p bgcolor=\"fd84\" height=\"1\"></p></s>", true),
            ("<r style=\"/**/Background-Color:URL(http:)#faa\"><a bgcolor=\";\" height=\"0\"></a></r>", true),
            ("<b bgcolor=\"#ffffff\" height=\"1\"></b><font bgcolor=\"#fafafa\" width=\"2px\"></font>", false),
            // Faint translucent text at the hidden threshold.
            ("<d bgcolor=\"crr\u{1}\"><e style=\"color/**/:&quot;&quot;&quot;&quot;#08be3e17\">x</e></d>", false),
        ] {
            let out = adapt_for_dark(html, remote);
            assert_eq!(adapt_for_dark(&out, remote), out, "{html}");
        }
        for keep in [
            // An escape with its white space continues a string past a newline.
            "<style>{text-decoration-color/**/:\"\\1\n\"\"#cAbD}</style>",
            "<style>{color:'\\1\n#B68}</style>",
            // `.url(` is a delimiter and a url; `<!--url(` a CDO and a url.
            "<style>{-webkit-text-fill-color:.url(#cee9}</style>",
            "<p style=\"border-left-color:x<!--url(https://x.example/a#fff);\">x</p>",
            // An at-keyword is not a colour name.
            "<p style=\"outline-color:x(03px8px&amp;#@tan,none)\">x</p>",
            // `5.url(` and `100%url(` are a number and a url.
            "<style>{-webkit-text-fill-color:5.url(#7AB}</style>",
            "<p style=\"column-rule:#C3cAf,URL(data:x)/100%url(https://x.example/a#fff);\">x</p>",
        ] {
            assert_eq!(adapt(keep), keep);
        }
        // White space before a comma is no combinator (this panicked).
        let out = stable("<style>.ghost\\:hov\\# ,&amp;#xer, [data-ogsc] .hero{color : yellow}.k , .j{background:#ffffff}</style><p class=\"k\"><span style=\"color:#ffffff\">hidden</span></p>");
        assert!(out.contains("color : yellow") && !out.contains("<span style=\"color:#ffffff\">"), "{out}");
        // A space after a hex escape is part of it: two keep the tokens apart.
        let out = stable("<p style=\"text-shadow:0 0 1px\u{a0}\\63#dbED80B1\">x</p>");
        assert!(out.contains("&nbsp;\\63  rgba("), "{out}");
        let out = stable("<p style=\"color:@#8Ed5\">x</p>");
        assert!(out.contains("@ rgba("), "{out}");
        // Nothing is appended after an escaped `;` or a non-breaking space
        // (CSS name characters), or appended twice.
        for style in ["e&lt;\\;", "\u{a0}", "\u{85}", "x:y \\;"] {
            for html in [format!("<td bgcolor=\"#ffffff\"><img src=\"{PNG}\" style=\"{style}\"></td>"), format!("<td bgcolor=\"#ffc107\"><a href=\"https://x.example\" style=\"{style}\">x</a></td>")] {
                let out = stable(&html);
                assert!(out.matches("color:#").count() <= 1 && !out.contains("\u{a0}color") && !out.contains("\u{a0}background"), "{out}");
            }
        }
    }

    #[test]
    fn light_scheme_only_turns_the_senders_dark_rules_off() {
        let html = "<style>.a{color:#000}@media (prefers-color-scheme: dark){.a{color:#fff}}@media screen and ( prefers-color-scheme : light ), print{.b{color:#000}}@media not all and (prefers-color-scheme:dark){.c{color:red}}</style><p class=\"a\">x</p>";
        let out = light_scheme_only(html);
        assert_eq!(out, "<style>.a{color:#000}@media (max-width:0px){.a{color:#fff}}@media screen and (min-width:0px), print{.b{color:#000}}@media not all and (max-width:0px){.c{color:red}}</style><p class=\"a\">x</p>");
        // Nothing else is touched.
        for keep in ["<p>(prefers-color-scheme: dark)</p>", "<style>.a{content:'(prefers-color-scheme: dark)'}</style>", "<style>@media (max-width:600px){.a{color:#000}}</style>"] {
            assert_eq!(light_scheme_only(keep), keep);
        }
    }

    // ---- round-3 review ----

    fn hex_after(out: &str, prefix: &str) -> Rgba {
        color::parse(after(out, prefix)).unwrap_or_else(|| panic!("{prefix}: {out}"))
    }

    #[test]
    fn text_on_a_kept_accent_keeps_the_light_designs_colour() {
        // Text with no colour of its own on a neon coupon of a dark design
        // keeps the light frame's (not the dark frame's white); a white
        // chip in it stays white under that colour.
        let out = stable(r###"<div style="background:#111111"><div style="background:#ffd400;padding:12px">Use code <span style="background:#ffffff">SAVE20</span></div></div>"###);
        assert!(out.contains(r###"<div style="background:#ffd400;padding:12px;color:#242424">Use code <span style="background:#ffffff">"###), "{out}");
        // An inherited colour is the light design's.
        let out = stable(r###"<table bgcolor="#111111"><tr><td style="color:#eeeeee"><font color="#333333"><table bgcolor="#00e5ff"><tr><td>x</td></tr></table></font></td></tr></table>"###);
        assert!(out.contains(r###"<table style="color:#333333" bgcolor="#00e5ff">"###), "{out}");
        // Where a rule colours text other than through a lone class (a tag,
        // a descendant or a link rule), the accent is adapted like a box:
        // the rule's adapted colour reads on it.
        for sheet in ["p{color:#222222}", ".box p{color:#241c15}", "a{color:#0645ad}", ".cta a{color:#041e42}"] {
            let html = format!(r###"<style>{sheet}</style><table bgcolor="#0b1d3a"><tr><td><table class="box cta" bgcolor="#4dd0e1"><tr><td><p>Members save 30%</p><a href="https://x.example">Join</a></td></tr></table></td></tr></table>"###);
            let out = stable(&html);
            let fill = hex_after(&out, r###"class="box cta" bgcolor=""###);
            assert!(color::is_dark(fill) && !out.contains("color:#242424"), "{sheet}: {out}");
        }
    }

    #[test]
    fn a_pinned_colour_is_adapted_again_on_a_box_adapted_again() {
        // .copy is pinned (#333) over the light picture; a white card in it
        // is darkened, so its text gets #333 adapted for the card.
        let html = format!(r###"<style>.copy{{color:#333333}}</style><p class="copy">x</p><table><tr><td bgcolor="#f5efe6" style="background-image:url({PNG})"><div class="copy">Over <div style="background:#ffffff;padding:10px">A white card</div></div><div><span style="color:#444444">Inline <b style="background:#ffffff">card</b></span></div></td></tr></table>"###);
        let out = stable(&html);
        assert!(out.contains(r###"<div style="color:#333333" class="copy">Over"###), "{out}");
        let card = after(&out, "<div style=\"background:");
        let ink = hex_after(&out, "padding:10px;color:");
        assert!(color::contrast(ink, color::parse(card).unwrap()) >= 4.5, "{out}");
        let b = out.split("<b style=\"").nth(1).unwrap();
        let (bg, fg) = (hex_after(b, "background:"), hex_after(b, "color:"));
        assert!(color::contrast(fg, bg) >= 4.5, "{out}");
    }

    #[test]
    fn spacers_get_no_plate() {
        for html in [
            format!(r###"<p><img src="{PNG}" width="600" height="30" alt=""><img src="{PNG}" width="20" height="20"></p>"###),
            r###"<p><img src="https://x.example/images/spacer.gif" width="600" height="30" alt="space"><img src="https://x.example/1x1.png" width="20" height="20" alt="x"><img src="https://x.example/blank.gif?v=2" width="20" height="20" alt="x"></p>"###.to_owned(),
        ] {
            assert!(!adapt_for_dark(&html, true).contains("<img style"), "{html}");
        }
        // A linked image with no alt text is content (a social icon).
        let out = stable(&format!(r###"<p><a href="https://x.example"><img src="{PNG}" width="24" height="24" alt=""></a></p>"###));
        assert!(out.contains("<img style=\"background-image:radial-gradient("), "{out}");
    }

    #[test]
    fn a_tile_with_an_icon_is_no_mark() {
        // A white icon on a black tile keeps the tile; a rule holding a
        // spacer is still a mark.
        let out = stable(&format!(r###"<table bgcolor="#ffffff"><tr><td width="48" height="48" bgcolor="#000000"><img src="{PNG}" width="24" height="24" alt=""></td><td width="48" height="48" bgcolor="#0b3d91"><img src="{PNG}" width="24" height="24" alt="ok"></td><td width="40" height="4" bgcolor="#333333"><img src="https://x.example/spacer.gif" width="40" height="4"></td></tr></table>"###));
        assert!(out.contains(r###"bgcolor="#000000""###) && out.contains(r###"bgcolor="#0b3d91""###), "{out}");
        let rule = hex_after(&out, r###"height="4" bgcolor=""###);
        assert!(color::contrast(rule, color::map_background(color::WHITE)) >= 3.0, "{out}");
    }

    #[test]
    fn backgrounds_the_adapter_cant_read_get_no_plate() {
        // var() with no fallback may be dark: a white logo there gets no
        // white plate (a class, inline, or a class inside a white table).
        for html in [
            format!(r###"<style>:root{{--hero:#0a0a0a}}.hero{{background:var(--hero)}}</style><div class="hero"><img src="{PNG}" width="200" height="50" alt="NORTHWIND"></div>"###),
            format!(r###"<div style="background:var(--hero)"><p><img src="{PNG}" width="200" height="50" alt="NORTHWIND"></p></div>"###),
            format!(r###"<style>.hero{{background-color:var(--hero)}}</style><table bgcolor="#ffffff"><tr><td class="hero"><img src="{PNG}" width="200" height="50" alt="NORTHWIND"></td></tr></table>"###),
        ] {
            let out = stable(&html);
            assert!(!out.contains("<img style"), "{out}");
        }
        // With a fallback colour it is read as that; an opaque known box
        // inside is known again.
        let out = stable(&format!(r###"<div style="background:var(--x, #ffffff)"><img src="{PNG}" width="200" height="50" alt="a"></div><div style="background:var(--hero)"><div style="background:#ffffff"><img src="{PNG}" width="200" height="50" alt="b"></div></div>"###));
        assert_eq!(out.matches("<img style=\"background-color:#ffffff").count(), 2, "{out}");
    }

    #[test]
    fn chart_bars_and_their_legend_match() {
        // Bars of one series are one colour whatever their height, and the
        // highlighted dark bar is as visible as its legend swatch.
        let bars: String = [50, 66, 113].iter().map(|h| format!(r###"<td width="14" height="{h}" bgcolor="#00a1e0"></td>"###)).collect();
        let html = format!(r###"<table bgcolor="#f7f7f7"><tr>{bars}<td width="14" height="65" bgcolor="#00205b"></td></tr></table><table bgcolor="#ffffff"><tr><td width="12" height="12" bgcolor="#00a1e0"></td><td width="12" height="12" bgcolor="#00205b"></td></tr></table>"###);
        let out = stable(&html);
        let series: Vec<&str> = out.split("bgcolor=\"").skip(1).map(|s| &s[..7]).collect();
        // chart, 3 bars, the dark bar, legend table, 2 swatches
        assert!(series[1] == series[2] && series[2] == series[3] && series[3] == series[6], "{out}");
        assert_eq!(series[4], series[7], "{out}");
        assert!(color::contrast(color::parse(series[4]).unwrap(), color::parse(series[0]).unwrap()) >= 2.0, "{out}");
        // An empty cell of a coloured band (its colour is a content cell's)
        // stays the band's colour.
        let out = stable(r###"<table bgcolor="#ffffff"><tr><td width="20" height="80" bgcolor="#00205b"></td><td height="80" bgcolor="#00205b" style="color:#ffffff">Account update</td></tr></table>"###);
        assert_eq!(out.matches("bgcolor=\"#00205b\"").count(), 2, "{out}");
    }

    #[test]
    fn a_senders_important_dark_class_background_wins_over_an_inline_one() {
        // Litmus pattern: the inline white is light mode's; the frame paints
        // the class's #1f1f1f, so a white dark-only wordmark is no hidden
        // text and the cell's own text is adapted for the dark colour.
        let html = r###"<style>.dark-img{display:none}@media (prefers-color-scheme: dark){.body-bg{background-color:#1f1f1f !important}.dark-img{display:block !important}}</style><table class="body-bg" style="background-color:#ffffff;color:#1b1b1b"><tr><td><div class="dark-img"><span style="color:#ffffff">Microsoft</span></div><p style="color:#2a2a2a">Code</p></td></tr></table>"###;
        let out = stable(html);
        assert!(out.contains(r###"<span style="color:#ffffff">Microsoft"###), "{out}");
        let bg = color::parse("#1f1f1f").unwrap();
        for prefix in ["<p style=\"color:", "background-color:#2f2f2f;color:"] {
            assert!(color::contrast(hex_after(&out, prefix), bg) >= 4.5, "{prefix}: {out}");
        }
        // Without !important the inline background wins, as before.
        let out = stable(&html.replace(" !important}.dark-img", "}.dark-img"));
        assert!(!out.contains(r###"<span style="color:#ffffff">"###), "{out}");
    }

    #[test]
    fn plates_carry_the_light_designs_alt_text_colour() {
        // An image that fails to load shows its alt text on the plate: in
        // the light design's colour (its own, or the one it inherits).
        let html = r###"<table bgcolor="#ffffff"><tr><td style="color:#333333"><img src="https://x.example/logo.png" width="150" height="40" alt="Brightline"><img src="https://x.example/hero.png" width="300" height="60" alt="Tips" style="color:#222222 !important"></td></tr></table>"###;
        let out = adapt_for_dark(html, true);
        assert_eq!(adapt_for_dark(&out, true), out);
        assert!(out.contains(r###"<img style="background-color:#ffffff;color:#333333" src="https://x.example/logo.png""###), "{out}");
        assert!(out.ends_with(r###"background-color:#ffffff;color:#222222 !important"></td></tr></table>"###), "{out}");
    }

    #[test]
    fn a_background_picture_with_no_colour_gets_its_light_backdrop() {
        // A transparent wordmark or icon drawn as a CSS background: the
        // light colour it was designed on goes under it, and text there
        // keeps the light design's colour.
        let html = format!(r###"<table bgcolor="#ffffff"><tr><td style="background:url({PNG}) no-repeat;padding-left:160px;color:#555555">Statement</td><td style="background:url({PNG}) no-repeat">Inherited</td></tr></table><p><span style="background-image:url({PNG});padding-left:20px;color:#333333">Verified</span></p>"###);
        let out = stable(&html);
        assert!(out.contains("no-repeat;padding-left:160px;color:#555555;background-color:#ffffff\">"), "{out}");
        assert!(out.contains("no-repeat;background-color:#ffffff;color:#242424\">Inherited"), "{out}");
        assert!(out.contains("padding-left:20px;color:#333333;background-color:#ffffff\">Verified"), "{out}");
        // Not with a colour of its own, on a dark background, or when it
        // won't load.
        for keep in [
            format!(r###"<table bgcolor="#ffffff"><tr><td style="background:#f5efe6 url({PNG})">x</td></tr></table>"###),
            format!(r###"<table bgcolor="#111111"><tr><td style="background:url({PNG})">x</td></tr></table>"###),
        ] {
            assert!(!adapt(&keep).contains("background-color"), "{keep}");
        }
        assert!(!adapt(r###"<td style="background:url(https://x.example/a.png)">x</td>"###).contains("background-color"));
    }

    /// Synthetic fixtures modelled on common real-world patterns.
    pub(crate) const FIXTURE_RECRUITER: &str = r##"<body style="margin:0;background-color:#ffffff;"><table width="600" style="font-family:Arial,sans-serif;"><tr><td style="padding:16px 24px;border-bottom:1px solid #e5e5e5;"><span style="font-size:22px;color:#cc0000;">TalentGrid</span> <span style="font-size:12px;color:#999999;">Job alert</span></td></tr><tr><td style="background-color:#eef4fb;padding:20px;border-left:4px solid #1a73e8;"><h1 style="color:#333333;">Hi Jordan, 5 new jobs match</h1><p style="color:#666666;">Based on your saved search.</p></td></tr><tr><td><a href="https://jobs.example.com/1" style="color:#1a0dab;">Senior Rust Engineer</a><br><span style="color:#333;">Northwind Robotics</span><br><span style="color:#666;">Full-time</span></td></tr><tr><td style="background-color:#cc0000;"><a href="https://jobs.example.com/all" style="color:#ffffff;">See all 5 jobs</a></td></tr><tr><td style="background:#f7f7f7;color:#888888;">You are receiving this because you created a job alert.</td></tr></table></body>"##;

    pub(crate) const FIXTURE_NEWSLETTER: &str = r##"<style>
  body, .wrapper { background-color: #f4f4f4; }
  .container { background: #ffffff; width: 600px; }
  .hero { background-color: #14213d; background-image: url("https://news.example.com/hero.jpg"); color: #ffffff; }
  .content { color: #3c3c3c; font-size: 16px; }
  .muted { color: #8a8a8a; }
  .btn { background-color: #0b5cff; color: #ffffff !important; }
  @media only screen and (max-width: 620px) { .hero h1 { font-size: 24px !important; color: #ffffff !important; } }
</style><table class="wrapper" width="100%"><tr><td><table class="container"><tr><td class="hero"><h1>Faster builds are here</h1><a class="btn" href="https://news.example.com/read">Read the guide</a></td></tr><tr><td class="content"><p>The new CLI bundles formatting and linting.</p><p class="muted">Sponsored by Example Cloud.</p></td></tr></table></td></tr></table>"##;

    pub(crate) const FIXTURE_OUTLOOK: &str = r##"<html><head><style><!--
p.MsoNormal, li.MsoNormal, div.MsoNormal
	{margin:0in;
	font-size:11.0pt;
	font-family:"Calibri",sans-serif;}
a:link, span.MsoHyperlink
	{mso-style-priority:99;
	color:#0563C1;
	text-decoration:underline;}
--></style></head><body lang=EN-US link="#0563C1"><div class=WordSection1><p class=MsoNormal><span style='color:#1F497D'>Hi Jordan,<o:p></o:p></span></p><p class=MsoNormal><span style='font-size:9.0pt;font-family:"Arial",sans-serif;color:#7F7F7F'>Program Manager | Contoso Ltd.</span></p><p class=MsoNormal><span style='background:yellow;mso-highlight:yellow'>Deadline is Friday.</span></p></div></body></html>"##;

    pub(crate) const FIXTURE_GITHUB: &str = r##"<div style="font-family:-apple-system,'Segoe UI',sans-serif;color:#24292f;"><p><a href="https://code.example.com/u/octavia" style="color:#0969da;">@octavia</a> requested changes.</p><div style="border:1px solid #d0d7de;"><div style="background-color:#f6f8fa;color:#57606a;">In src/parser.rs:</div><pre style="background-color:#ffffff;"><span style="color:#116329;background-color:#dafbe1;">+ let n = buf.len().min(MAX);</span></pre><code style="background-color:rgba(175,184,193,0.2);">MAX</code></div></div>"##;
}
