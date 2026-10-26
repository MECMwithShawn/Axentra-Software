//! The embedded header may request only these native window operations.
use serde::Deserialize;
use tao::window::ResizeDirection;
use wry::http::Request;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum Action {
    State,
    Minimize,
    ToggleMaximize,
    /// Borderless fullscreen on and off (F11).
    Fullscreen,
    Close,
    /// Close without asking again (the page already handled unsaved changes).
    Quit,
    /// The page has started closing (saving or discarding), so the window keeps waiting for it.
    Closing,
    Drag,
    Resize { edge: Edge },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
    Nw,
}

impl From<Edge> for ResizeDirection {
    fn from(edge: Edge) -> Self {
        match edge {
            Edge::N => Self::North,
            Edge::Ne => Self::NorthEast,
            Edge::E => Self::East,
            Edge::Se => Self::SouthEast,
            Edge::S => Self::South,
            Edge::Sw => Self::SouthWest,
            Edge::W => Self::West,
            Edge::Nw => Self::NorthWest,
        }
    }
}

pub fn parse(request: &Request<String>) -> Option<Action> {
    let uri = request.uri();
    let local = uri.scheme_str() == Some(if cfg!(windows) { "http" } else { "axn" })
        && uri.authority().map(|a| a.as_str())
            == Some(if cfg!(windows) { "axn.localhost" } else { "localhost" })
        && uri.path() == "/";
    if !local || request.body().len() > 128 {
        return None;
    }
    // Serde's internally tagged unit variants otherwise ignore extra fields.
    let value: serde_json::Value = serde_json::from_str(request.body()).ok()?;
    let fields = value.as_object()?;
    let resizing = fields.get("action")?.as_str()? == "resize";
    if fields.len() != if resizing { 2 } else { 1 }
        || (resizing && !fields.contains_key("edge"))
    {
        return None;
    }
    serde_json::from_value(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_commands_require_local_page_and_known_action() {
        let request = |url: &str, body: &str| {
            Request::builder().uri(url).body(body.to_owned()).unwrap()
        };
        assert!(matches!(
            parse(&request(&format!("{}/", crate::ORIGIN), r#"{"action":"minimize"}"#)),
            Some(Action::Minimize)
        ));
        for url in ["https://example.com/", "axn://localhost.evil/", "axn://localhost:80/", "axx://localhost/", "about:blank"] {
            assert!(parse(&request(url, r#"{"action":"close"}"#)).is_none());
        }
        for body in [
            r#"{"action":"launch","exe":"sh"}"#,
            r#"{"action":"resize","edge":"bad"}"#,
            r#"{"action":"close","extra":true}"#,
        ] {
            assert!(parse(&request(&format!("{}/", crate::ORIGIN), body)).is_none());
        }
        assert!(matches!(parse(&request(&format!("{}/", crate::ORIGIN), r#"{"action":"quit"}"#)), Some(Action::Quit)));
        assert!(matches!(parse(&request(&format!("{}/", crate::ORIGIN), r#"{"action":"closing"}"#)), Some(Action::Closing)));
        assert!(matches!(parse(&request(&format!("{}/", crate::ORIGIN), r#"{"action":"fullscreen"}"#)), Some(Action::Fullscreen)));
        assert!(matches!(
            parse(&request(&format!("{}/", crate::ORIGIN), r#"{"action":"resize","edge":"se"}"#)),
            Some(Action::Resize { edge: Edge::Se })
        ));
    }
}
