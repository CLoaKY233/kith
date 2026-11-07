//! `kith join https://<console>/j/<code>`: fetches the invite link a Console hands out for
//! a device code, over HTTPS checked against the web's public certificate authorities.

use core::time::Duration;
use std::sync::Arc;

use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::ServerName;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

use crate::exec::CliError;

/// How long fetching may take, from connecting to the last byte.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

/// The largest response read; an invite link is far smaller.
const MAX_RESPONSE_LEN: u64 = 64 * 1024;

/// What every invite link starts with.
const INVITE_PREFIX: &str = "kith://join/";

/// Whether `link` is a web address rather than an invite link.
pub(crate) fn is_web_link(link: &str) -> bool {
    let lower = link.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Fetches the invite link behind a Console's `https://` address.
pub(crate) async fn invite_link(url: &str) -> Result<String, CliError> {
    let target = Target::parse(url)?;
    let body = tokio::time::timeout(FETCH_TIMEOUT, get(&target))
        .await
        .map_err(|_| CliError(format!("{} did not answer in time", target.host)))??;
    find_invite(&body).ok_or_else(|| {
        CliError(format!(
            "{} sent no invite; the code may have expired, so ask for a new one",
            target.host
        ))
    })
}

/// Where to fetch from.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    host: String,
    port: u16,
    path: String,
}

impl Target {
    fn parse(url: &str) -> Result<Self, CliError> {
        let url = url.trim();
        let rest = url
            .get(..8)
            .filter(|scheme| scheme.eq_ignore_ascii_case("https://"))
            .and_then(|_| url.get(8..))
            .ok_or_else(|| {
                CliError(format!(
                    "`{url}` is not an https:// address; kith fetches invites only over https"
                ))
            })?;
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (authority, path) = rest.split_at(split);
        let path = match path.find('#') {
            Some(fragment) => path.get(..fragment).unwrap_or_default(),
            None => path,
        };
        let bad = || CliError(format!("`{url}` is not a valid address"));
        if authority.is_empty() || authority.contains('@') {
            return Err(bad());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !port.contains(']') => {
                (host, port.parse::<u16>().map_err(|_| bad())?)
            }
            _ => (authority, 443),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() || !path.chars().all(|c| c.is_ascii_graphic()) {
            return Err(bad());
        }
        Ok(Self {
            host: host.to_owned(),
            port,
            path: if path.starts_with('/') {
                path.to_owned()
            } else {
                format!("/{path}")
            },
        })
    }

    /// The `Host` header's value.
    fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == 443 {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }
}

/// A plain HTTP/1.0 `GET`, so the answer is neither chunked nor kept alive.
async fn get(target: &Target) -> Result<String, CliError> {
    let failed = |error: &dyn core::fmt::Display| {
        CliError(format!(
            "cannot fetch the invite from {}: {error}",
            target.host
        ))
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| failed(&error))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = ServerName::try_from(target.host.clone()).map_err(|error| failed(&error))?;
    let tcp = TcpStream::connect((target.host.as_str(), target.port))
        .await
        .map_err(|error| failed(&error))?;
    let mut tls = TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
        .map_err(|error| failed(&error))?;
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nAccept: text/plain\r\nUser-Agent: kith/{}\r\n\r\n",
        target.path,
        target.authority(),
        env!("CARGO_PKG_VERSION"),
    );
    tls.write_all(request.as_bytes())
        .await
        .map_err(|error| failed(&error))?;
    let mut response = Vec::new();
    // A server may close without TLS's close_notify once everything is sent.
    let read = (&mut tls)
        .take(MAX_RESPONSE_LEN)
        .read_to_end(&mut response)
        .await;
    if let Err(error) = read
        && response.is_empty()
    {
        return Err(failed(&error));
    }
    let text = String::from_utf8_lossy(&response);
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).unwrap_or_default();
    if status != "200" {
        let line = head.lines().next().unwrap_or_default();
        return Err(CliError(format!(
            "{} answered `{line}`; the code may be wrong or expired",
            target.host
        )));
    }
    Ok(body.to_owned())
}

/// The first invite link in `body`, whether it is plain text, JSON or HTML.
fn find_invite(body: &str) -> Option<String> {
    let start = body.to_ascii_lowercase().find(INVITE_PREFIX)?;
    let link: String = body
        .get(start..)?
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '-' | '_' | '='))
        .collect();
    (link.len() > INVITE_PREFIX.len()).then_some(link)
}

#[cfg(test)]
mod tests {
    use super::{Target, find_invite, is_web_link};

    #[test]
    fn only_https_addresses_are_fetched() {
        let target = Target::parse("HTTPS://vpn.example.com/j/ABC123").unwrap();
        assert_eq!(
            target,
            Target {
                host: "vpn.example.com".into(),
                port: 443,
                path: "/j/ABC123".into()
            }
        );
        let other = Target::parse("https://[::1]:8443/j/x#top").unwrap();
        assert_eq!((other.host.as_str(), other.port), ("::1", 8443));
        assert_eq!(
            (other.path.as_str(), other.authority()),
            ("/j/x", "[::1]:8443".into())
        );
        assert!(is_web_link("http://vpn.example.com/j/ABC123"));
        assert!(!is_web_link("kith://join/ABC"));
        for bad in [
            "http://vpn.example.com/j/ABC123",
            "https://",
            "https://user@vpn.example.com/j/x",
            "https://vpn.example.com:99999/j/x",
            "https://vpn.example.com/j/a b",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_invite_is_found_in_text_or_json() {
        assert_eq!(
            find_invite("kith://join/ABCD234\n").as_deref(),
            Some("kith://join/ABCD234")
        );
        assert_eq!(
            find_invite(r#"{"invite_link":"KITH://JOIN/ABCD234"}"#).as_deref(),
            Some("KITH://JOIN/ABCD234")
        );
        assert_eq!(find_invite("no such code"), None);
        assert_eq!(find_invite("kith://join/"), None);
    }
}
