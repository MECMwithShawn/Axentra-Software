//! "Looking up your account…" (§1.3): provider presets by domain, the
//! Microsoft Entra tenant probe, Thunderbird-style autoconfig over HTTPS,
//! then a guess for the manual form. Each network step has a 3 s timeout.

use std::time::Duration;

use crate::config::{Provider, Security, ServerConfig, domain_of, guess_servers, preset, provider_for_domain};

const STEP_TIMEOUT: Duration = Duration::from_secs(3);

/// What detection concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub provider: Provider,
    /// Suggested servers for IMAP providers (shown before any password is sent).
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// Where the answer came from: "preset", "microsoft", "autoconfig" or "guess".
    pub source: &'static str,
}

fn tag(block: &str, name: &str) -> Option<String> {
    let start = block.find(&format!("<{name}>"))? + name.len() + 2;
    let end = block[start..].find(&format!("</{name}>"))? + start;
    Some(block[start..end].trim().to_string())
}

fn server_block<'a>(xml: &'a str, element: &str, kind: &str) -> Option<&'a str> {
    let mut rest = xml;
    while let Some(pos) = rest.find(&format!("<{element}")) {
        let after = &rest[pos..];
        let head_end = after.find('>')?;
        let head = &after[..head_end];
        let close = format!("</{element}>");
        let end = after.find(&close)?;
        if head.contains(&format!("type=\"{kind}\"")) || head.contains(&format!("type='{kind}'")) {
            return Some(&after[..end]);
        }
        rest = &after[end + close.len()..];
    }
    None
}

fn parse_server(block: &str, email: &str) -> Option<ServerConfig> {
    let host = tag(block, "hostname")?;
    let port: u16 = tag(block, "port")?.parse().ok()?;
    let security = match tag(block, "socketType")?.to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        // Never offer plain text.
        _ => return None,
    };
    let local = email.split('@').next().unwrap_or_default();
    let username = match tag(block, "username").as_deref() {
        Some("%EMAILLOCALPART%") => local.to_string(),
        Some(u) if !u.contains('%') => u.to_string(),
        _ => String::new(),
    };
    Some(ServerConfig { host, port, security, username })
}

/// Parse a `clientConfig` (config-v1.1.xml) document.
pub fn parse_autoconfig(xml: &str, email: &str) -> Option<(ServerConfig, ServerConfig)> {
    let imap = parse_server(server_block(xml, "incomingServer", "imap")?, email)?;
    let smtp = parse_server(server_block(xml, "outgoingServer", "smtp")?, email)?;
    Some((imap, smtp))
}

async fn get_text(http: &reqwest::Client, url: &str) -> Option<(u16, String)> {
    let resp = tokio::time::timeout(STEP_TIMEOUT, http.get(url).send()).await.ok()?.ok()?;
    let status = resp.status().as_u16();
    let text = tokio::time::timeout(STEP_TIMEOUT, resp.text()).await.ok()?.ok()?;
    Some((status, text))
}

/// Detect the provider for an address.
pub async fn detect(http: &reqwest::Client, email: &str) -> Detection {
    let domain = domain_of(email).unwrap_or_default();
    if let Some(provider) = provider_for_domain(&domain) {
        let p = preset(provider).expect("every known provider has a preset");
        return Detection { provider, imap: p.imap, smtp: p.smtp, source: "preset" };
    }
    let (imap, smtp) = guess_servers(&domain);
    // A domain that belongs to an Entra tenant: try Microsoft first.
    let probe = format!("https://login.microsoftonline.com/{domain}/v2.0/.well-known/openid-configuration");
    if let Some((200, _)) = get_text(http, &probe).await {
        return Detection { provider: Provider::MicrosoftWork, imap, smtp, source: "microsoft" };
    }
    let address: String = url::form_urlencoded::byte_serialize(email.trim().as_bytes()).collect();
    for url in [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={address}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ] {
        if let Some((200, xml)) = get_text(http, &url).await
            && let Some((imap, smtp)) = parse_autoconfig(&xml, email)
        {
            return Detection { provider: Provider::Imap, imap, smtp, source: "autoconfig" };
        }
    }
    Detection { provider: Provider::Imap, imap, smtp, source: "guess" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autoconfig_document() {
        let xml = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
    <incomingServer type="imap">
      <hostname>mail.example.org</hostname><port>143</port><socketType>STARTTLS</socketType>
      <username>%EMAILLOCALPART%</username><authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>smtp.example.org</hostname><port>465</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;
        let (imap, smtp) = parse_autoconfig(xml, "ana@example.org").unwrap();
        assert_eq!(imap, ServerConfig { host: "mail.example.org".into(), port: 143, security: Security::StartTls, username: "ana".into() });
        assert_eq!(smtp, ServerConfig { host: "smtp.example.org".into(), port: 465, security: Security::Tls, username: String::new() });
        let plain = xml.replace("STARTTLS", "plain");
        assert_eq!(parse_autoconfig(&plain, "ana@example.org"), None, "plain text is never offered");
    }

    #[tokio::test]
    async fn presets_skip_the_network() {
        let d = detect(&reqwest::Client::new(), "someone@gmail.com").await;
        assert_eq!((d.provider, d.source, d.imap.host.as_str()), (Provider::Google, "preset", "imap.gmail.com"));
        let d = detect(&reqwest::Client::new(), "someone@hotmail.com").await;
        assert_eq!(d.provider, Provider::MicrosoftPersonal);
    }
}
