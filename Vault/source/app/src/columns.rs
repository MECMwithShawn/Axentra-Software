//! Columns of the Classic layout's entry table: which are shown, in a fixed order. Saved with the
//! other non-secret preferences (column names only, never values).

use eframe::egui;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Col {
    Username,
    Website,
    Category,
    Group,
    Tags,
    Created,
    Modified,
    Expires,
    Notes,
    /// A custom (string) field, by name.
    Custom(String),
    /// Why the entry is listed: health issues, search matches, archive date.
    Status,
}

/// Built-in columns in display order; custom fields go between Notes and Status.
pub fn built_in() -> [Col; 10] {
    [
        Col::Username,
        Col::Website,
        Col::Category,
        Col::Group,
        Col::Tags,
        Col::Created,
        Col::Modified,
        Col::Expires,
        Col::Notes,
        Col::Status,
    ]
}

pub fn defaults() -> Vec<Col> {
    vec![
        Col::Username,
        Col::Website,
        Col::Category,
        Col::Modified,
        Col::Status,
    ]
}

impl Col {
    pub fn key(&self) -> String {
        match self {
            Col::Username => "username".into(),
            Col::Website => "website".into(),
            Col::Category => "category".into(),
            Col::Group => "group".into(),
            Col::Tags => "tags".into(),
            Col::Created => "created".into(),
            Col::Modified => "modified".into(),
            Col::Expires => "expires".into(),
            Col::Notes => "notes".into(),
            Col::Status => "status".into(),
            Col::Custom(name) => format!("custom:{name}"),
        }
    }

    pub fn parse(key: &str) -> Option<Col> {
        if let Some(name) = key.strip_prefix("custom:") {
            return (!name.is_empty()).then(|| Col::Custom(name.to_owned()));
        }
        built_in().into_iter().find(|c| c.key() == key)
    }

    pub fn label(&self) -> String {
        match self {
            Col::Username => "Username".into(),
            Col::Website => "Website".into(),
            Col::Category => "Category".into(),
            Col::Group => "Group".into(),
            Col::Tags => "Tags".into(),
            Col::Created => "Created".into(),
            Col::Modified => "Modified".into(),
            Col::Expires => "Expires".into(),
            Col::Notes => "Notes".into(),
            Col::Status => "Status".into(),
            Col::Custom(name) => name.clone(),
        }
    }

    pub fn width(&self) -> f32 {
        match self {
            Col::Username => 210.0,
            Col::Website => 170.0,
            Col::Category | Col::Group | Col::Tags => 130.0,
            Col::Created | Col::Modified | Col::Expires => 90.0,
            Col::Notes => 220.0,
            Col::Custom(_) => 150.0,
            Col::Status => 160.0,
        }
    }

    fn rank(&self) -> usize {
        match self {
            Col::Custom(_) => 9,
            Col::Status => 10,
            other => built_in().iter().position(|c| c == other).unwrap_or(0),
        }
    }
}

/// Shown columns in display order.
pub fn ordered(mut cols: Vec<Col>) -> Vec<Col> {
    cols.sort_by(|a, b| {
        a.rank()
            .cmp(&b.rank())
            .then_with(|| a.label().cmp(&b.label()))
    });
    cols.dedup();
    cols
}

pub fn from_text(text: &str) -> Vec<Col> {
    ordered(text.lines().filter_map(|l| Col::parse(l.trim())).collect())
}

pub fn to_text(cols: &[Col]) -> String {
    cols.iter().map(Col::key).collect::<Vec<_>>().join("\n")
}

/// Menu config that keeps the column list open while several boxes are ticked.
pub fn stay_open() -> egui::containers::menu::MenuConfig {
    egui::containers::menu::MenuConfig::new()
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
}

/// Checkboxes for every column; `custom_names` are the custom fields used in this vault.
/// Returns the new column set when something was toggled.
pub fn menu(ui: &mut egui::Ui, shown: &[Col], custom_names: &[String]) -> Option<Vec<Col>> {
    let mut next: Option<Vec<Col>> = None;
    let mut toggle = |ui: &mut egui::Ui, col: Col| {
        let mut on = shown.contains(&col);
        if ui.checkbox(&mut on, col.label()).changed() {
            let mut cols: Vec<Col> = shown.iter().filter(|c| **c != col).cloned().collect();
            if on {
                cols.push(col);
            }
            next = Some(ordered(cols));
        }
    };
    ui.label(egui::RichText::new("Title is always shown").weak().small());
    for col in built_in() {
        toggle(ui, col);
    }
    if !custom_names.is_empty() {
        ui.separator();
        ui.label(egui::RichText::new("Custom fields").strong());
        for name in custom_names {
            toggle(ui, Col::Custom(name.clone()));
        }
    }
    ui.separator();
    if ui.button("Reset to default columns").clicked() {
        next = Some(defaults());
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_orders() {
        let cols = vec![
            Col::Status,
            Col::Custom("Account #".into()),
            Col::Username,
            Col::Expires,
        ];
        let text = to_text(&cols);
        assert_eq!(
            from_text(&text),
            [
                Col::Username,
                Col::Expires,
                Col::Custom("Account #".into()),
                Col::Status
            ]
        );
        assert_eq!(from_text("bogus\nwebsite\ncustom:"), [Col::Website]);
    }
}
