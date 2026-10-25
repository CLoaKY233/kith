//! HTTP/1.1 upgrade exchange (RFC 6455 §4). Strict and bounded: one head of at most
//! [`MAX_HEAD`] bytes and [`MAX_HEADERS`] header lines, CRLF only, no obsolete folding.

use data_encoding::BASE64;

use crate::Error;
use crate::sha1::sha1;

/// Largest accepted request or response head, terminator included.
pub(crate) const MAX_HEAD: usize = 8192;
const MAX_HEADERS: usize = 64;
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// What the server needs from a request head.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request<'a> {
    pub(crate) path: &'a str,
    /// The `Sec-WebSocket-Key`, present only when the request is a valid version-13 upgrade.
    pub(crate) upgrade_key: Option<&'a str>,
}

/// `Sec-WebSocket-Accept` for a key (§4.2.2).
pub(crate) fn accept_value(key: &str) -> String {
    let mut input = String::with_capacity(key.len() + GUID.len());
    input.push_str(key);
    input.push_str(GUID);
    BASE64.encode(&sha1(input.as_bytes()))
}

/// A fresh `Sec-WebSocket-Key` from 16 nonce bytes.
pub(crate) fn key_from_nonce(nonce: &[u8; 16]) -> String {
    BASE64.encode(nonce)
}

fn check_field(value: &str, allow_space: bool, what: &'static str) -> Result<(), Error> {
    let ok = !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() || (allow_space && b == b' '));
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidField(what))
    }
}

/// Validates an upgrade path: absolute, visible ASCII only.
pub(crate) fn check_path(path: &str) -> Result<(), Error> {
    check_field(path, false, "path")?;
    if path.starts_with('/') {
        Ok(())
    } else {
        Err(Error::InvalidField("path"))
    }
}

/// The client's upgrade request.
pub(crate) fn client_request(
    host: &str,
    path: &str,
    user_agent: &str,
    key: &str,
) -> Result<String, Error> {
    check_field(host, false, "host")?;
    check_path(path)?;
    check_field(user_agent, true, "user agent")?;
    Ok(format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: {user_agent}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    ))
}

/// The server's 101 answer.
pub(crate) fn switching_response(key: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_value(key)
    )
}

const NOT_FOUND_BODY: &str = "<!DOCTYPE html>\n<html>\n<head><title>404 Not Found</title></head>\n\
<body>\n<h1>404 Not Found</h1>\n</body>\n</html>\n";

/// The plain 404 every request other than the configured upgrade gets.
pub(crate) fn not_found_response() -> String {
    format!(
        "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{NOT_FOUND_BODY}",
        NOT_FOUND_BODY.len()
    )
}

struct Head<'a> {
    start_line: &'a str,
    headers: Vec<(&'a str, &'a str)>,
}

impl<'a> Head<'a> {
    fn get(&self, name: &str) -> Option<&'a str> {
        let mut found = self
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name));
        let &(_, value) = found.next()?;
        // A repeated singleton header is ambiguous; treat it as absent.
        if found.next().is_some() {
            None
        } else {
            Some(value)
        }
    }

    fn has_token(&self, name: &str, token: &str) -> bool {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .flat_map(|(_, v)| v.split(','))
            .any(|t| t.trim().eq_ignore_ascii_case(token))
    }
}

/// Splits off a complete head. `Ok(None)` means more bytes are needed.
fn split_head(buf: &[u8]) -> Result<Option<(Head<'_>, usize)>, Error> {
    let window = buf.get(..MAX_HEAD.min(buf.len())).unwrap_or(buf);
    let Some(end) = window.windows(4).position(|w| w == b"\r\n\r\n") else {
        return if buf.len() >= MAX_HEAD {
            Err(Error::Handshake("head too large"))
        } else {
            Ok(None)
        };
    };
    let text = window.get(..end).ok_or(Error::Handshake("head"))?;
    let text = core::str::from_utf8(text).map_err(|_| Error::Handshake("head is not text"))?;
    let mut lines = text.split("\r\n");
    let start_line = lines.next().unwrap_or_default();
    let mut headers = Vec::new();
    for line in lines {
        if headers.len() == MAX_HEADERS {
            return Err(Error::Handshake("too many headers"));
        }
        if line.bytes().any(|b| b.is_ascii_control() && b != b'\t') {
            return Err(Error::Handshake("control character in header"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or(Error::Handshake("header without colon"))?;
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Error::Handshake("bad header name"));
        }
        headers.push((name, value.trim_matches([' ', '\t'])));
    }
    if start_line.bytes().any(|b| b.is_ascii_control()) {
        return Err(Error::Handshake("control character in start line"));
    }
    Ok(Some((
        Head {
            start_line,
            headers,
        },
        end + 4,
    )))
}

/// Parses a request head; returns it and the bytes it used.
pub(crate) fn parse_request(buf: &[u8]) -> Result<Option<(Request<'_>, usize)>, Error> {
    let Some((head, used)) = split_head(buf)? else {
        return Ok(None);
    };
    let mut parts = head.start_line.split(' ');
    let (Some(method), Some(path), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(Error::Handshake("bad request line"));
    };
    if version != "HTTP/1.1" || !path.starts_with('/') {
        return Err(Error::Handshake("bad request line"));
    }
    let key = head
        .get("Sec-WebSocket-Key")
        .filter(|k| BASE64.decode(k.as_bytes()).is_ok_and(|raw| raw.len() == 16));
    let is_upgrade = method == "GET"
        && head.has_token("Upgrade", "websocket")
        && head.has_token("Connection", "upgrade")
        && head.get("Sec-WebSocket-Version") == Some("13")
        && head.get("Host").is_some();
    Ok(Some((
        Request {
            path,
            upgrade_key: key.filter(|_| is_upgrade),
        },
        used,
    )))
}

/// Checks the server's answer to `key`; returns the bytes it used.
pub(crate) fn parse_response(buf: &[u8], key: &str) -> Result<Option<usize>, Error> {
    let Some((head, used)) = split_head(buf)? else {
        return Ok(None);
    };
    let mut parts = head.start_line.splitn(3, ' ');
    let (Some("HTTP/1.1"), Some(code)) = (parts.next(), parts.next()) else {
        return Err(Error::Handshake("bad status line"));
    };
    let code: u16 = code
        .parse()
        .map_err(|_| Error::Handshake("bad status code"))?;
    if code != 101 {
        return Err(Error::Rejected(code));
    }
    let accepted = head.has_token("Upgrade", "websocket")
        && head.has_token("Connection", "upgrade")
        && head.get("Sec-WebSocket-Accept") == Some(accept_value(key).as_str());
    if accepted {
        Ok(Some(used))
    } else {
        Err(Error::Handshake("upgrade not confirmed"))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn accept_value_matches_rfc_6455_example() {
        assert_eq!(
            accept_value("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    proptest! {
        #[test]
        fn head_parsers_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
            drop(parse_request(&bytes));
            drop(parse_response(&bytes, "dGhlIHNhbXBsZSBub25jZQ=="));
        }
    }
}
