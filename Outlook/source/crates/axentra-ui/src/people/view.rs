//! What the contact list shows: the selected navigation node filtered by
//! the search text, sorted, and split under index headings
//! (docs/spec/people-todo.md 2.5). Pure functions, so they are unit tested.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use axentra_core::*;
use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, Utc};

use super::text::{fold, index_heading, phone_digits};

/// A node of the navigation tree.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    Favorites,
    AllContacts,
    AllLists,
    /// A contact folder; "" is the default "Contacts" folder.
    Folder(String),
    Category(String),
    Suggested,
}

impl Node {
    pub fn title(&self) -> String {
        match self {
            Node::Favorites => "Favorites".into(),
            Node::AllContacts => "All contacts".into(),
            Node::AllLists => "All contact lists".into(),
            Node::Folder(f) if f.is_empty() => "Contacts".into(),
            Node::Folder(f) => f.clone(),
            Node::Category(c) => c.clone(),
            Node::Suggested => "Suggested".into(),
        }
    }

    pub fn shows_contact(&self, c: &Contact) -> bool {
        match self {
            Node::Favorites => c.is_favorite,
            Node::AllContacts => true,
            Node::AllLists | Node::Suggested => false,
            Node::Folder(f) => &c.folder == f,
            Node::Category(cat) => c.categories.iter().any(|x| x == cat),
        }
    }

    /// Lists live in the default "Contacts" folder (2.9).
    pub fn shows_list(&self, l: &ContactList) -> bool {
        match self {
            Node::Favorites => l.is_favorite,
            Node::AllContacts | Node::AllLists => true,
            Node::Folder(f) => f.is_empty(),
            Node::Category(_) | Node::Suggested => false,
        }
    }

    /// Serialised form for the "last node" preference.
    pub fn key(&self) -> String {
        match self {
            Node::Favorites => "favorites".into(),
            Node::AllContacts => "all".into(),
            Node::AllLists => "lists".into(),
            Node::Folder(f) => format!("folder:{f}"),
            Node::Category(c) => format!("category:{c}"),
            Node::Suggested => "suggested".into(),
        }
    }

    pub fn from_key(k: &str) -> Node {
        match k {
            "favorites" => Node::Favorites,
            "lists" => Node::AllLists,
            "suggested" => Node::Suggested,
            _ => {
                if let Some(f) = k.strip_prefix("folder:") {
                    Node::Folder(f.to_string())
                } else if let Some(c) = k.strip_prefix("category:") {
                    Node::Category(c.to_string())
                } else {
                    Node::AllContacts
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortBy {
    #[default]
    FirstName,
    LastName,
    Company,
    RecentlyAdded,
    HomeCity,
    WorkCity,
}

impl SortBy {
    pub const MENU: [SortBy; 4] = [SortBy::FirstName, SortBy::LastName, SortBy::Company, SortBy::RecentlyAdded];
    pub const EXTRA: [SortBy; 2] = [SortBy::HomeCity, SortBy::WorkCity];

    pub fn label(self) -> &'static str {
        match self {
            SortBy::FirstName => "First name",
            SortBy::LastName => "Last name",
            SortBy::Company => "Company",
            SortBy::RecentlyAdded => "Recently added",
            SortBy::HomeCity => "Home city",
            SortBy::WorkCity => "Work city",
        }
    }

    /// Sort button label: "By first name".
    pub fn button_label(self) -> String {
        format!("By {}", self.label().to_lowercase())
    }

    pub fn key(self) -> &'static str {
        match self {
            SortBy::FirstName => "first",
            SortBy::LastName => "last",
            SortBy::Company => "company",
            SortBy::RecentlyAdded => "recent",
            SortBy::HomeCity => "home_city",
            SortBy::WorkCity => "work_city",
        }
    }

    pub fn from_key(k: &str) -> SortBy {
        [SortBy::FirstName, SortBy::LastName, SortBy::Company, SortBy::RecentlyAdded, SortBy::HomeCity, SortBy::WorkCity]
            .into_iter()
            .find(|s| s.key() == k)
            .unwrap_or_default()
    }
}

/// Something selectable in the list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Item {
    Contact(ContactId),
    List(ContactListId),
    /// A "Suggested" person, by lower-case address.
    Suggested(String),
}

impl Item {
    /// DOM id of the row, used to move focus.
    pub fn dom_id(&self) -> String {
        match self {
            Item::Contact(id) => format!("ax-people-row-c{id}"),
            Item::List(id) => format!("ax-people-row-l{id}"),
            Item::Suggested(a) => format!("ax-people-row-s{}", a.bytes().map(|b| format!("{b:02x}")).collect::<String>()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Heading(String),
    Contact(Rc<Contact>),
    List(Rc<ContactList>),
    Suggested(Rc<EmailAddress>, i64),
}

impl Row {
    pub fn item(&self) -> Option<Item> {
        match self {
            Row::Heading(_) => None,
            Row::Contact(c) => Some(Item::Contact(c.id)),
            Row::List(l) => Some(Item::List(l.id)),
            Row::Suggested(a, _) => Some(Item::Suggested(a.address.to_lowercase())),
        }
    }
}

/// The list pane's rows plus lookups computed once per rebuild, so that
/// rendering and keyboard handling never scan the whole list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListModel {
    pub rows: Vec<Row>,
    /// Number of selectable rows (the header count).
    pub count: usize,
    /// Distinct headings in order, with their row index.
    pub headings: Vec<(String, usize)>,
    /// Selectable items in list order.
    items: Vec<Item>,
    /// Item -> (index in `items`, index in `rows`).
    index: HashMap<Item, (usize, usize)>,
}

impl ListModel {
    pub fn new(rows: Vec<Row>, headings: Vec<(String, usize)>) -> Self {
        let mut items = Vec::with_capacity(rows.len());
        let mut index = HashMap::with_capacity(rows.len());
        for (r, row) in rows.iter().enumerate() {
            if let Some(item) = row.item() {
                index.insert(item.clone(), (items.len(), r));
                items.push(item);
            }
        }
        Self { count: items.len(), rows, headings, items, index }
    }

    pub fn items(&self) -> impl Iterator<Item = Item> + '_ {
        self.items.iter().cloned()
    }

    pub fn item_list(&self) -> &[Item] {
        &self.items
    }

    /// Position of an item among the selectable items.
    pub fn item_index(&self, item: &Item) -> Option<usize> {
        self.index.get(item).map(|(i, _)| *i)
    }

    /// Row index of an item (rows include headings).
    pub fn row_index(&self, item: &Item) -> Option<usize> {
        self.index.get(item).map(|(_, r)| *r)
    }
}

/// Name as shown in lists for the "Display contacts by" setting, with the
/// fallbacks of 2.5.2: company, primary email, first phone, "(No name)".
pub fn display_name(c: &Contact, last_first: bool) -> String {
    let given = c.given_name.trim();
    let surname = c.surname.trim();
    if last_first && (!given.is_empty() || !surname.is_empty()) {
        return match (surname.is_empty(), given.is_empty()) {
            (false, false) => format!("{surname}, {given}"),
            (false, true) => surname.to_string(),
            _ => given.to_string(),
        };
    }
    if !c.display_name.trim().is_empty() {
        return c.display_name.trim().to_string();
    }
    let joined = [given, c.middle_name.trim(), surname].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    if !joined.is_empty() {
        return joined;
    }
    fallback_name(c)
}

fn fallback_name(c: &Contact) -> String {
    if !c.company.trim().is_empty() {
        return c.company.trim().to_string();
    }
    if let Some(e) = c.primary_email() {
        return e.to_string();
    }
    if let Some(p) = c.phones.first() {
        return p.value.clone();
    }
    "(No name)".into()
}

/// Mobile phone, else work phone, else any.
pub fn main_phone(c: &Contact) -> Option<&str> {
    c.phones
        .iter()
        .find(|p| p.label == "Mobile")
        .or_else(|| c.phones.iter().find(|p| p.label == "Work"))
        .or_else(|| c.phones.first())
        .map(|p| p.value.as_str())
}

/// Case- and accent-insensitive search over the fields listed in 2.5.1.
/// Every whitespace-separated term must match some field.
pub fn contact_matches(c: &Contact, query: &str) -> bool {
    let terms: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    if terms.is_empty() {
        return true;
    }
    let mut hay = vec![
        fold(&c.display_name),
        fold(&c.given_name),
        fold(&c.surname),
        fold(&c.middle_name),
        fold(&c.nickname),
        fold(&c.company),
        fold(&c.notes),
        fold(&c.job_title),
    ];
    hay.extend(c.emails.iter().map(|e| fold(&e.value)));
    let phones: Vec<String> = c.phones.iter().map(|p| phone_digits(&p.value)).collect();
    terms.iter().all(|t| {
        let t_digits = phone_digits(t);
        hay.iter().any(|h| h.contains(t.as_str()))
            || (t_digits.len() >= 3 && t_digits.len() * 2 >= t.chars().count() && phones.iter().any(|p| p.contains(&t_digits)))
    })
}

pub fn list_matches(l: &ContactList, query: &str) -> bool {
    let terms: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
    let name = fold(&l.name);
    let desc = fold(&l.description);
    terms.iter().all(|t| name.contains(t.as_str()) || desc.contains(t.as_str()))
}

fn city(c: &Contact, label: &str) -> String {
    c.addresses.iter().find(|a| a.label == label).map(|a| a.city.trim().to_string()).unwrap_or_default()
}

/// The primary sort key (what the index heading comes from) for a contact.
fn primary_key(c: &Contact, sort: SortBy) -> String {
    let name = display_name(c, false);
    match sort {
        SortBy::FirstName => if c.given_name.trim().is_empty() { name } else { c.given_name.trim().to_string() },
        SortBy::LastName => if c.surname.trim().is_empty() { name } else { c.surname.trim().to_string() },
        SortBy::Company => c.company.trim().to_string(),
        SortBy::HomeCity => city(c, "Home"),
        SortBy::WorkCity => city(c, "Work"),
        SortBy::RecentlyAdded => name,
    }
}

fn secondary_key(c: &Contact, sort: SortBy) -> String {
    match sort {
        SortBy::FirstName => format!("{} {}", c.surname, display_name(c, false)),
        SortBy::LastName => format!("{} {}", c.given_name, display_name(c, false)),
        _ => display_name(c, false),
    }
}

/// Recently-added bucket for a creation time.
pub fn added_bucket(t: DateTime<Utc>, today: NaiveDate) -> &'static str {
    let d = t.with_timezone(&Local).date_naive();
    let week_start = today - Duration::days(today.weekday().num_days_from_sunday() as i64);
    let month_start = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    if d >= today {
        "Today"
    } else if d == today - Duration::days(1) {
        "Yesterday"
    } else if d >= week_start {
        "This week"
    } else if d >= month_start {
        "This month"
    } else {
        "Older"
    }
}

/// One sortable entry before headings are inserted.
struct Entry {
    row: Row,
    heading: String,
    hash_last: bool,
    key: String,
    key2: String,
    created: DateTime<Utc>,
    favorite_order: Option<i64>,
}

/// Inputs for [`build`]; borrowed so the caller keeps ownership of the data.
pub struct BuildInput<'a> {
    pub node: &'a Node,
    pub contacts: &'a [Contact],
    pub lists: &'a [ContactList],
    pub suggested: &'a [(EmailAddress, i64)],
    pub query: &'a str,
    pub sort: SortBy,
    pub today: NaiveDate,
}

pub fn build(input: BuildInput<'_>) -> ListModel {
    let BuildInput { node, contacts, lists, suggested, query, sort, today } = input;
    let query = query.trim();
    let mut entries: Vec<Entry> = Vec::new();
    for c in contacts.iter().filter(|c| node.shows_contact(c) && contact_matches(c, query)) {
        let key = primary_key(c, sort);
        let heading = if sort == SortBy::RecentlyAdded { added_bucket(c.created_at, today).to_string() } else { index_heading(&key) };
        entries.push(Entry {
            hash_last: heading == "#",
            heading,
            key: fold(&key),
            key2: fold(&secondary_key(c, sort)),
            created: c.created_at,
            favorite_order: c.favorite_order,
            row: Row::Contact(Rc::new(c.clone())),
        });
    }
    for l in lists.iter().filter(|l| node.shows_list(l) && (query.is_empty() || list_matches(l, query))) {
        let heading = match sort {
            SortBy::RecentlyAdded => added_bucket(l.created_at, today).to_string(),
            // Lists have no company or address: they file under "#" like empty keys.
            SortBy::Company | SortBy::HomeCity | SortBy::WorkCity => "#".to_string(),
            _ => index_heading(&l.name),
        };
        entries.push(Entry {
            hash_last: heading == "#",
            heading,
            key: fold(&l.name),
            key2: String::new(),
            created: l.created_at,
            favorite_order: l.favorite_order,
            row: Row::List(Rc::new(l.clone())),
        });
    }
    if *node == Node::Suggested {
        for (addr, n) in suggested {
            let name = addr.display().to_string();
            if !query.is_empty() && !fold(&format!("{name} {}", addr.address)).contains(&fold(query)) {
                continue;
            }
            entries.push(Entry {
                hash_last: false,
                heading: String::new(),
                key: String::new(),
                key2: String::new(),
                created: DateTime::<Utc>::default(),
                favorite_order: Some(-*n),
                row: Row::Suggested(Rc::new(addr.clone()), *n),
            });
        }
    }

    let flat = matches!(node, Node::Favorites | Node::Suggested);
    if flat {
        // Dragged order first, then alphabetical (2.10); Suggested by count.
        entries.sort_by(|a, b| match (a.favorite_order, b.favorite_order) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.key.cmp(&b.key).then_with(|| a.key2.cmp(&b.key2)),
        });
    } else if sort == SortBy::RecentlyAdded {
        entries.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.key.cmp(&b.key)));
    } else {
        entries.sort_by(|a, b| {
            a.hash_last.cmp(&b.hash_last).then_with(|| a.key.cmp(&b.key)).then_with(|| a.key2.cmp(&b.key2))
        });
    }

    let mut rows = Vec::with_capacity(entries.len() + 30);
    let mut headings = Vec::new();
    let mut last: Option<String> = None;
    for e in entries {
        if !flat && last.as_deref() != Some(e.heading.as_str()) {
            headings.push((e.heading.clone(), rows.len()));
            rows.push(Row::Heading(e.heading.clone()));
            last = Some(e.heading);
        }
        rows.push(e.row);
    }
    ListModel::new(rows, headings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: i64, given: &str, surname: &str, company: &str) -> Contact {
        Contact {
            id,
            given_name: given.into(),
            surname: surname.into(),
            display_name: [given, surname].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" "),
            company: company.into(),
            ..Default::default()
        }
    }

    fn names(m: &ListModel) -> Vec<String> {
        m.rows
            .iter()
            .map(|r| match r {
                Row::Heading(h) => format!("[{h}]"),
                Row::Contact(c) => display_name(c, false),
                Row::List(l) => format!("<{}>", l.name),
                Row::Suggested(a, _) => a.address.clone(),
            })
            .collect()
    }

    fn input<'a>(node: &'a Node, contacts: &'a [Contact], lists: &'a [ContactList], query: &'a str, sort: SortBy) -> BuildInput<'a> {
        BuildInput { node, contacts, lists, suggested: &[], query, sort, today: NaiveDate::from_ymd_opt(2026, 10, 19).unwrap() }
    }

    #[test]
    fn sorts_with_headings_accents_and_hash_last() {
        let contacts = vec![
            contact(1, "Émile", "Dubois", ""),
            contact(2, "ada", "Lovelace", "Analytical"),
            contact(3, "Ben", "Carter", ""),
            contact(4, "", "", "42 Club"),
            contact(5, "Álvaro", "Zed", ""),
        ];
        let lists = vec![ContactList { id: 9, name: "Book club".into(), ..Default::default() }];
        let m = build(input(&Node::AllContacts, &contacts, &lists, "", SortBy::FirstName));
        assert_eq!(
            names(&m),
            ["[A]", "ada Lovelace", "Álvaro Zed", "[B]", "Ben Carter", "<Book club>", "[E]", "Émile Dubois", "[#]", "42 Club"]
        );
        assert_eq!(m.count, 6);
        assert_eq!(m.headings.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(), ["A", "B", "E", "#"]);

        let by_last = build(input(&Node::AllContacts, &contacts, &[], "", SortBy::LastName));
        assert_eq!(names(&by_last), ["[C]", "Ben Carter", "[D]", "Émile Dubois", "[L]", "ada Lovelace", "[Z]", "Álvaro Zed", "[#]", "42 Club"]);

        let by_company = build(input(&Node::AllContacts, &contacts, &[], "", SortBy::Company));
        assert_eq!(names(&by_company)[..2], ["[A]", "ada Lovelace"]);
        assert_eq!(names(&by_company)[2], "[#]");
    }

    #[test]
    fn filters_by_node_and_query() {
        let mut a = contact(1, "Ada", "Lovelace", "");
        a.is_favorite = true;
        a.emails = vec![LabeledValue { label: "Work".into(), value: "ada@example.com".into() }];
        a.phones = vec![LabeledValue { label: "Mobile".into(), value: "+1 (415) 555-0123".into() }];
        let mut b = contact(2, "José", "Núñez", "");
        b.folder = "Family".into();
        b.categories = vec!["VIP".into()];
        let contacts = vec![a, b];
        let lists = vec![ContactList { id: 3, name: "Team".into(), is_favorite: true, ..Default::default() }];
        let fav = build(input(&Node::Favorites, &contacts, &lists, "", SortBy::FirstName));
        assert_eq!(names(&fav), ["Ada Lovelace", "<Team>"], "favorites have no headings");
        assert_eq!(build(input(&Node::Folder("Family".into()), &contacts, &lists, "", SortBy::FirstName)).count, 1);
        assert_eq!(build(input(&Node::Folder(String::new()), &contacts, &lists, "", SortBy::FirstName)).count, 2);
        assert_eq!(build(input(&Node::AllLists, &contacts, &lists, "", SortBy::FirstName)).count, 1);
        assert_eq!(build(input(&Node::Category("VIP".into()), &contacts, &lists, "", SortBy::FirstName)).count, 1);
        let all = build(input(&Node::AllContacts, &contacts, &lists, "", SortBy::FirstName));
        assert_eq!(all.item_index(&Item::List(3)), Some(2));
        assert_eq!(all.row_index(&Item::List(3)), Some(5), "after the A, J and T headings");
        assert_eq!(all.item_index(&Item::Contact(99)), None);
        let q = |s| build(input(&Node::AllContacts, &contacts, &lists, s, SortBy::FirstName)).count;
        assert_eq!(q("jose nunez"), 1, "accent-insensitive");
        assert_eq!(q("EXAMPLE.com"), 1);
        assert_eq!(q("5550123"), 1, "phone digits only");
        assert_eq!(q("415 555"), 1);
        assert_eq!(q("team"), 1);
        assert_eq!(q("zzz"), 0);
    }

    #[test]
    fn favorites_respect_drag_order() {
        let mut a = contact(1, "Ada", "", "");
        let mut b = contact(2, "Ben", "", "");
        let mut c = contact(3, "Cy", "", "");
        a.is_favorite = true;
        b.is_favorite = true;
        c.is_favorite = true;
        c.favorite_order = Some(0);
        b.favorite_order = Some(1);
        let m = build(input(&Node::Favorites, &[a, b, c], &[], "", SortBy::FirstName));
        assert_eq!(names(&m), ["Cy", "Ben", "Ada"]);
    }

    #[test]
    fn recently_added_buckets() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap(); // Thursday
        let at = |m: u32, d: u32| Local.with_ymd_and_hms(2026, m, d, 12, 0, 0).unwrap().with_timezone(&Utc);
        use chrono::TimeZone;
        assert_eq!(added_bucket(at(10, 22), today), "Today");
        assert_eq!(added_bucket(at(10, 21), today), "Yesterday");
        assert_eq!(added_bucket(at(10, 18), today), "This week");
        assert_eq!(added_bucket(at(10, 2), today), "This month");
        assert_eq!(added_bucket(at(9, 30), today), "Older");
    }

    #[test]
    fn display_name_orders_and_fallbacks() {
        let c = contact(1, "Ada", "Lovelace", "");
        assert_eq!(display_name(&c, false), "Ada Lovelace");
        assert_eq!(display_name(&c, true), "Lovelace, Ada");
        let company_only = Contact { company: "Bayview Dental".into(), ..Default::default() };
        assert_eq!(display_name(&company_only, true), "Bayview Dental");
        let phone_only = Contact { phones: vec![LabeledValue { label: "Mobile".into(), value: "555".into() }], ..Default::default() };
        assert_eq!(display_name(&phone_only, false), "555");
        assert_eq!(display_name(&Contact::default(), false), "(No name)");
    }

    #[test]
    fn node_and_sort_keys_round_trip() {
        for n in [Node::Favorites, Node::AllContacts, Node::AllLists, Node::Folder(String::new()), Node::Folder("Family".into()), Node::Category("VIP".into()), Node::Suggested] {
            assert_eq!(Node::from_key(&n.key()), n);
        }
        for s in SortBy::MENU.into_iter().chain(SortBy::EXTRA) {
            assert_eq!(SortBy::from_key(s.key()), s);
        }
        assert_eq!(SortBy::FirstName.button_label(), "By first name");
    }
}
