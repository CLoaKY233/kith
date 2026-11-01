//! The tray's own way into kith: a `kith-ui-core` router called in process, with its own
//! session, so the tray reads exactly what the page reads and never talks to the daemon
//! another way. Nothing listens on a port for it.

use std::collections::VecDeque;
use std::io;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use http_body_util::BodyExt as _;
use kith_ui_core::wire::{ApiErrorBody, ApiErrorCode, SessionIn};
use kith_ui_core::{Config, DEFAULT_PORT, Source, Token, router};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tower::ServiceExt as _;

use crate::sse::{Event, Parser};

/// The port the router believes it serves, for its `Host` and `Origin` checks.
const PORT: u16 = DEFAULT_PORT;

/// Why a call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    /// The HTTP status.
    pub(crate) status: StatusCode,
    /// The server's reason, when it gave one.
    pub(crate) code: Option<ApiErrorCode>,
    /// The server's words.
    pub(crate) message: String,
}

impl Failure {
    fn internal(message: &impl ToString) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: Some(ApiErrorCode::Internal),
            message: message.to_string(),
        }
    }
}

impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::internal(&error)
    }
}

/// A signed-in client of one router.
#[derive(Debug, Clone)]
pub(crate) struct Client {
    router: Router,
    cookie: String,
}

impl Client {
    /// Builds a router over `source` and signs in to it with a fresh one-time token.
    pub(crate) async fn start(source: Source) -> Result<Self, Failure> {
        let token = Token::random()?;
        let hex = token.hex();
        let router = router(Config {
            source,
            port: PORT,
            token,
            assets: None,
            dev_origins: Vec::new(),
        })?;
        let mut client = Self {
            router,
            cookie: String::new(),
        };
        let response = client
            .send(
                Method::POST,
                "/api/session",
                Some(&SessionIn { token: hex }),
            )
            .await?;
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .ok_or_else(|| Failure::internal(&"the session answer set no cookie"))?;
        cookie.clone_into(&mut client.cookie);
        Ok(client)
    }

    async fn send<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> Result<axum::response::Response, Failure> {
        let mut request = Request::builder()
            .method(method.clone())
            .uri(path)
            .header(header::HOST, format!("127.0.0.1:{PORT}"));
        if method == Method::POST || path.starts_with("/api/events") {
            request = request.header(header::ORIGIN, format!("http://127.0.0.1:{PORT}"));
        }
        if !self.cookie.is_empty() {
            request = request.header(header::COOKIE, &self.cookie);
        }
        let body = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(body).map_err(|error| Failure::internal(&error))?)
            }
            None => Body::empty(),
        };
        let request = request
            .body(body)
            .map_err(|error| Failure::internal(&error))?;
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map_err(|error| Failure::internal(&error))?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| Failure::internal(&error))?
            .to_bytes();
        Err(match serde_json::from_slice::<ApiErrorBody>(&bytes) {
            Ok(error) => Failure {
                status,
                code: Some(error.code),
                message: error.message,
            },
            Err(_) => Failure {
                status,
                code: None,
                message: String::from_utf8_lossy(&bytes).into_owned(),
            },
        })
    }

    /// `GET path`, read as JSON.
    pub(crate) async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Failure> {
        let response = self.send::<()>(Method::GET, path, None).await?;
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| Failure::internal(&error))?
            .to_bytes();
        serde_json::from_slice(&bytes).map_err(|error| Failure::internal(&error))
    }

    /// `POST path` with a JSON body; the answer's body is not needed.
    pub(crate) async fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<(), Failure> {
        self.send(Method::POST, path, Some(body)).await.map(drop)
    }

    /// Opens `/api/events`, resuming after `last` (`<boot>:<seq>`) when given.
    pub(crate) async fn events(&self, last: Option<&str>) -> Result<Events, Failure> {
        let path = match last {
            Some(last) => format!("/api/events?last={last}"),
            None => "/api/events".to_owned(),
        };
        let response = self.send::<()>(Method::GET, &path, None).await?;
        Ok(Events {
            body: response.into_body(),
            parser: Parser::default(),
            ready: VecDeque::new(),
        })
    }
}

/// An open event stream.
#[derive(Debug)]
pub(crate) struct Events {
    body: Body,
    parser: Parser,
    ready: VecDeque<Event>,
}

impl Events {
    /// The next event, or `None` once the stream has ended.
    pub(crate) async fn next(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.ready.pop_front() {
                return Some(event);
            }
            let frame = self.body.frame().await?.ok()?;
            if let Ok(data) = frame.into_data() {
                self.ready.extend(self.parser.push(&data));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::BootIn;
    use crate::tests::office;

    #[tokio::test]
    async fn the_tray_signs_in_and_reads_what_the_page_reads() {
        let client = Client::start(Source::Fixture(Box::new(office())))
            .await
            .expect("signs in");
        let boot: BootIn = client.get("/api/boot").await.expect("reads the snapshot");
        assert_eq!(boot.status.networks.len(), 1);

        let refused = client.get::<BootIn>("/api/nothing-here").await;
        assert!(refused.is_err());

        let mut events = client.events(None).await.expect("opens the stream");
        let first = tokio::time::timeout(core::time::Duration::from_secs(5), events.next())
            .await
            .expect("an event within a tick")
            .expect("the stream is open");
        assert_eq!(first.name, "tick");
    }
}
