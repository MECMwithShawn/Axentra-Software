//! The Settings tree (docs/spec/shell.md §7.2) and the search index.

use crate::state::SettingsPage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Accounts,
    General,
    Mail,
    Calendar,
    People,
}

impl Category {
    pub const ALL: [Category; 5] = [Category::Accounts, Category::General, Category::Mail, Category::Calendar, Category::People];

    pub fn label(self) -> &'static str {
        match self {
            Category::Accounts => "Accounts",
            Category::General => "General",
            Category::Mail => "Mail",
            Category::Calendar => "Calendar",
            Category::People => "People",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Category::Accounts => "person",
            Category::General => "settings",
            Category::Mail => "mail",
            Category::Calendar => "calendar_ltr",
            Category::People => "people",
        }
    }

    pub fn pages(self) -> &'static [PageId] {
        use PageId::*;
        match self {
            Category::Accounts => &[EmailAccounts, Signatures, Categories, AutomaticReplies, SharedWithMe, Storage, Backup],
            Category::General => &[LanguageTime, Appearance, Notifications, Accessibility, Search, Offline, Privacy, DistributionGroups, About],
            Category::Mail => &[
                MessageList, ReadingPane, FolderPane, ComposeReply, Attachments, Rules, Sweep, Junk, QuickSteps, CustomizeActions,
                MessageHandling, Forwarding, ConditionalFormatting, SearchFolders, Smime, RetentionPolicies, Groups,
            ],
            Category::Calendar => &[CalendarView, EventsInvitations, Weather, SharedCalendars, CalendarActions, EventsFromEmail],
            Category::People => &[Contacts],
        }
    }

    pub fn from_settings_page(p: SettingsPage) -> Self {
        match p {
            SettingsPage::Accounts => Category::Accounts,
            SettingsPage::General => Category::General,
            SettingsPage::Mail => Category::Mail,
            SettingsPage::Calendar => Category::Calendar,
            SettingsPage::People => Category::People,
        }
    }

    pub fn settings_page(self) -> SettingsPage {
        match self {
            Category::Accounts => SettingsPage::Accounts,
            Category::General => SettingsPage::General,
            Category::Mail => SettingsPage::Mail,
            Category::Calendar => SettingsPage::Calendar,
            Category::People => SettingsPage::People,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageId {
    // Accounts
    EmailAccounts,
    Signatures,
    Categories,
    AutomaticReplies,
    SharedWithMe,
    Storage,
    Backup,
    // General
    LanguageTime,
    Appearance,
    Notifications,
    Accessibility,
    Search,
    Offline,
    Privacy,
    DistributionGroups,
    About,
    // Mail
    MessageList,
    ReadingPane,
    FolderPane,
    ComposeReply,
    Attachments,
    Rules,
    Sweep,
    Junk,
    QuickSteps,
    CustomizeActions,
    MessageHandling,
    Forwarding,
    ConditionalFormatting,
    SearchFolders,
    Smime,
    RetentionPolicies,
    Groups,
    // Calendar
    CalendarView,
    EventsInvitations,
    Weather,
    SharedCalendars,
    CalendarActions,
    EventsFromEmail,
    // People
    Contacts,
}

impl PageId {
    pub fn category(self) -> Category {
        Category::ALL.into_iter().find(|c| c.pages().contains(&self)).unwrap_or(Category::General)
    }

    pub fn title(self) -> &'static str {
        use PageId::*;
        match self {
            EmailAccounts => "Email accounts",
            Signatures => "Signatures",
            Categories => "Categories",
            AutomaticReplies => "Automatic replies",
            SharedWithMe => "Shared with me",
            Storage => "Storage",
            Backup => "Backup",
            LanguageTime => "Language and time",
            Appearance => "Appearance",
            Notifications => "Notifications",
            Accessibility => "Accessibility",
            Search => "Search",
            Offline => "Offline",
            Privacy => "Privacy and data",
            DistributionGroups => "Distribution groups",
            About => "About Axentra Outlook",
            MessageList => "Message list",
            ReadingPane => "Reading pane",
            FolderPane => "Folder pane",
            ComposeReply => "Compose and reply",
            Attachments => "Attachments",
            Rules => "Rules",
            Sweep => "Sweep",
            Junk => "Junk email",
            QuickSteps => "Quick steps",
            CustomizeActions | CalendarActions => "Customize actions",
            MessageHandling => "Message handling",
            Forwarding => "Forwarding",
            ConditionalFormatting => "Conditional formatting",
            SearchFolders => "Search folders",
            Smime => "S/MIME",
            RetentionPolicies => "Retention policies",
            Groups => "Groups",
            CalendarView => "View",
            EventsInvitations => "Events and invitations",
            Weather => "Weather",
            SharedCalendars => "Shared calendars",
            EventsFromEmail => "Events from email",
            Contacts => "Contacts",
        }
    }

    /// Pages that edit the settings draft and end with Save / Discard.
    /// The others act immediately (accounts, categories) or are read-only.
    pub fn uses_draft(self) -> bool {
        use PageId::*;
        matches!(
            self,
            Signatures
                | LanguageTime
                | Appearance
                | Notifications
                | Accessibility
                | Search
                | Offline
                | Privacy
                | MessageList
                | ReadingPane
                | FolderPane
                | ComposeReply
                | Attachments
                | MessageHandling
                | CalendarView
                | EventsInvitations
                | Contacts
        )
    }

    /// DOM id slug for an option label on this page (search highlight).
    pub fn anchor(label: &str) -> String {
        let slug: String = label.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
        format!("ax-setting-{}", slug.trim_matches('-'))
    }
}

/// Searchable option labels: (page, section, label). Page titles are
/// searchable too; "Layout" is an alias of Message list / Reading pane.
pub const INDEX: &[(PageId, &str, &str)] = &[
    (PageId::EmailAccounts, "Your accounts", "Add account"),
    (PageId::EmailAccounts, "Your accounts", "Remove account"),
    (PageId::Signatures, "Email signature", "Email signature"),
    (PageId::Signatures, "Select default signatures", "Include my signature on new messages"),
    (PageId::Signatures, "Select default signatures", "Include my signature on replies and forwards"),
    (PageId::Categories, "Categories", "Create category"),
    (PageId::Storage, "Storage", "Storage used on this device"),
    (PageId::Backup, "Back up your accounts", "Back up now"),
    (PageId::Backup, "Your backups", "Restore from a backup"),
    (PageId::Backup, "Where backups are saved", "OneDrive backup folder"),
    (PageId::Backup, "Backup password", "Backup password"),
    (PageId::Backup, "Schedule", "Back up automatically every month"),
    (PageId::Backup, "Schedule", "Keep the last"),
    (PageId::LanguageTime, "Time format", "Time format"),
    (PageId::LanguageTime, "Language", "Language"),
    (PageId::LanguageTime, "Current time zone", "Time zone"),
    (PageId::Appearance, "Theme", "Theme"),
    (PageId::Appearance, "Dark mode", "Dark mode"),
    (PageId::Appearance, "Dark mode", "Use a light background for messages"),
    (PageId::Appearance, "Density", "Density"),
    (PageId::Appearance, "Layout style", "Classic look"),
    (PageId::Appearance, "Title bar", "Use the system title bar"),
    (PageId::Appearance, "Accessibility", "High contrast"),
    (PageId::Notifications, "Notify me about", "Mail notifications"),
    (PageId::Notifications, "Notify me about", "Calendar reminders"),
    (PageId::Notifications, "Mail", "Play a sound"),
    (PageId::Notifications, "Mail", "Desktop notifications"),
    (PageId::Notifications, "Mail", "Only for Focused Inbox"),
    (PageId::Notifications, "Batching", "Combine multiple new-mail notifications"),
    (PageId::Accessibility, "Keyboard shortcuts", "Keyboard shortcuts"),
    (PageId::Accessibility, "Keyboard shortcuts", "Show key tips when Alt is pressed"),
    (PageId::Accessibility, "Motion", "Reduce motion"),
    (PageId::Accessibility, "Contrast", "High contrast"),
    (PageId::Accessibility, "Screen readers", "Announce new messages"),
    (PageId::Search, "Search results", "Search scope"),
    (PageId::Search, "Search results", "Include results from Deleted Items"),
    (PageId::Search, "Search history", "Clear search history"),
    (PageId::Offline, "Offline", "Enable offline email, calendar, and people"),
    (PageId::Offline, "Offline", "Folders to save"),
    (PageId::Offline, "Offline", "Days of email to save"),
    (PageId::Offline, "Offline", "Sync in the background"),
    (PageId::Privacy, "External images", "External images"),
    (PageId::About, "About", "Version"),
    (PageId::About, "About", "Licenses"),
    (PageId::MessageList, "Focused Inbox", "Sort messages into Focused and Other"),
    (PageId::MessageList, "Message list format", "Sender name first"),
    (PageId::MessageList, "Message list format", "Subject first"),
    (PageId::MessageList, "Text preview", "Show preview text"),
    (PageId::MessageList, "Sender image", "Show sender images"),
    (PageId::MessageList, "Attachment previews", "Show attachment previews"),
    (PageId::MessageList, "Layout", "Layout"),
    (PageId::ReadingPane, "Message organization", "Group messages by conversation"),
    (PageId::ReadingPane, "Reading pane position", "Reading pane position"),
    (PageId::ReadingPane, "After moving or deleting an item", "After moving or deleting an item"),
    (PageId::ReadingPane, "Conversation contents", "Show all messages from the selected conversation"),
    (PageId::ReadingPane, "Conversation order", "Newest on top"),
    (PageId::ReadingPane, "Deleted items in conversations", "Show deleted items"),
    (PageId::ReadingPane, "Layout", "Layout"),
    (PageId::FolderPane, "Folder order", "Sort folders A to Z"),
    (PageId::FolderPane, "Favorites", "Show favorites"),
    (PageId::FolderPane, "Folder pane", "Show the folder pane"),
    (PageId::ComposeReply, "Message format", "Default font"),
    (PageId::ComposeReply, "Undo send", "Undo send"),
    (PageId::Attachments, "Attachment previews", "Show attachment previews"),
    (PageId::Attachments, "Attachment reminder", "Warn me when I send a message that may be missing an attachment"),
    (PageId::Junk, "Blocked senders and domains", "Blocked senders and domains"),
    (PageId::Junk, "Safe senders and domains", "Safe senders and domains"),
    (PageId::Sweep, "Sweep rules", "Sweep rules"),
    (PageId::MessageHandling, "Mark as read", "Mark as read"),
    (PageId::MessageHandling, "Mark as read", "Mark as read after"),
    (PageId::MessageHandling, "Message options", "Empty Deleted Items when Axentra Outlook closes"),
    (PageId::CalendarView, "Calendar appearance", "First day of the week"),
    (PageId::CalendarView, "Calendar appearance", "Show week numbers"),
    (PageId::CalendarView, "Calendar appearance", "Default view"),
    (PageId::CalendarView, "Work hours and location", "Work days"),
    (PageId::CalendarView, "Work hours and location", "Work hours"),
    (PageId::CalendarView, "Time zones", "Time zone"),
    (PageId::EventsInvitations, "Events you create", "Default reminder"),
    (PageId::EventsInvitations, "Events you create", "Default event length"),
    (PageId::Contacts, "Contacts", "Sort contacts by"),
    (PageId::Contacts, "Contacts", "Display contacts as"),
];

/// A search hit: page, breadcrumb ("Category › Page › Section" for an
/// option, just the category for a page), option label.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub page: PageId,
    pub crumb: String,
    pub label: String,
}

/// Pages and options whose title, section or label contains `query`.
pub fn search(query: &str) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Hit> = Vec::new();
    for c in Category::ALL {
        for &p in c.pages() {
            if p.title().to_lowercase().contains(&q) {
                out.push(Hit { page: p, crumb: c.label().into(), label: p.title().into() });
            }
        }
    }
    for (page, section, label) in INDEX {
        let hay = format!("{section} {label}").to_lowercase();
        if hay.contains(&q) && !out.iter().any(|h| h.page == *page && h.label == *label) {
            let mut crumb = format!("{} › {}", page.category().label(), page.title());
            if *section != page.title() {
                crumb = format!("{crumb} › {section}");
            }
            out.push(Hit { page: *page, crumb, label: (*label).into() });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_belongs_to_one_category() {
        let mut seen = std::collections::HashSet::new();
        for c in Category::ALL {
            for p in c.pages() {
                assert!(seen.insert(*p), "{p:?} listed twice");
                assert_eq!(p.category(), c);
            }
        }
        assert_eq!(Category::from_settings_page(SettingsPage::Mail).pages()[0], PageId::MessageList);
    }

    #[test]
    fn search_finds_pages_options_and_the_layout_alias() {
        assert!(search("mark as read").iter().any(|h| h.page == PageId::MessageHandling));
        assert!(search("dark").iter().any(|h| h.page == PageId::Appearance));
        let layout: Vec<PageId> = search("layout").iter().map(|h| h.page).collect();
        assert!(layout.contains(&PageId::MessageList) && layout.contains(&PageId::ReadingPane));
        assert!(search("Signatures")[0].page == PageId::Signatures);
        assert!(search("onedrive").iter().any(|h| h.page == PageId::Backup));
        assert!(search("restore").iter().any(|h| h.page == PageId::Backup));
        assert!(search("title bar").iter().any(|h| h.page == PageId::Appearance));
        assert!(search("   ").is_empty());
    }

    #[test]
    fn breadcrumbs_name_each_level_once() {
        let hits = search("notif");
        let page = hits.iter().find(|h| h.label == "Notifications").unwrap();
        assert_eq!(page.crumb, "General");
        let option = hits.iter().find(|h| h.label == "Mail notifications").unwrap();
        assert_eq!(option.crumb, "General › Notifications › Notify me about");
        for h in search("e") {
            let parts: Vec<&str> = h.crumb.split(" › ").collect();
            assert!(parts.windows(2).all(|w| w[0] != w[1]), "{}", h.crumb);
        }
    }

    #[test]
    fn anchors_are_dom_safe() {
        assert_eq!(PageId::anchor("Use a light background for messages"), "ax-setting-use-a-light-background-for-messages");
        assert_eq!(PageId::anchor("S/MIME"), "ax-setting-s-mime");
    }
}
