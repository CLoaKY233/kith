//! The window's page server: the same `kith-ui-core` router `kith ui` serves, on a free
//! loopback port, started when the window opens and stopped when it closes. Each start
//! has its own one-time token, so the web view signs in exactly as a browser tab does and
//! never needs `invoke` for data.

use std::io;

use axum::Router;
use kith_ui_core::{Asset, Config, Source, Token, bind, router};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// The built page, or `fallback.html` when it was not built; see `build.rs`.
const ASSETS: &[Asset] = include!(concat!(env!("OUT_DIR"), "/assets.rs"));

/// A running page server. Dropping it stops the server.
#[derive(Debug)]
pub(crate) struct Page {
    /// The loopback port it listens on.
    pub(crate) port: u16,
    stop: Option<oneshot::Sender<()>>,
}

impl Drop for Page {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _already_stopped = stop.send(());
        }
    }
}

/// A page server that is bound but not yet serving.
#[derive(Debug)]
pub(crate) struct Bound {
    listener: TcpListener,
    app: Router,
    port: u16,
    token: String,
}

impl Bound {
    /// Binds a free loopback port and builds the router for it over `source`.
    pub(crate) async fn new(source: Source) -> io::Result<Self> {
        let listener = bind(0).await?;
        let port = listener.local_addr()?.port();
        let token = Token::random()?;
        let hex = token.hex();
        let app = router(Config {
            source,
            port,
            token,
            assets: Some(ASSETS),
            dev_origins: Vec::new(),
        })?;
        Ok(Self {
            listener,
            app,
            port,
            token: hex,
        })
    }

    /// The window's first address: `path` on this server, signing in with the one-time
    /// token. `join` carries an invite code to the page's join flow; both stay in the
    /// fragment, which never reaches the server.
    pub(crate) fn link(&self, path: &str, join: Option<&str>) -> String {
        link(self.port, path, &self.token, join)
    }

    /// Serves, on the current runtime, until the returned [`Page`] is dropped.
    pub(crate) fn serve(self) -> Page {
        let (stop, stopped) = oneshot::channel::<()>();
        tokio::spawn(async move {
            let _served = axum::serve(self.listener, self.app)
                .with_graceful_shutdown(async {
                    let _either = stopped.await;
                })
                .await;
        });
        Page {
            port: self.port,
            stop: Some(stop),
        }
    }
}

fn link(port: u16, path: &str, token: &str, join: Option<&str>) -> String {
    let join = join.map_or_else(String::new, |code| format!("&join={code}"));
    format!("http://127.0.0.1:{port}{path}#t={token}{join}")
}

/// A page path on the server at `port`, for a window that is already signed in.
pub(crate) fn address(port: u16, path: &str) -> String {
    format!("http://127.0.0.1:{port}{path}")
}

/// Whether the web view may load `url` itself: only this page server, by the address it
/// listens on. `localhost` is refused: it may resolve to `::1`, where another program can
/// listen on the same port, or be renamed in the hosts file. Everything else opens in the
/// person's browser or not at all.
pub(crate) fn stays_inside(url: &tauri::Url, port: u16) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(port)
        && url.username().is_empty()
        && url.password().is_none()
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpStream;

    use super::*;
    use crate::tests::office;

    async fn get(port: u16, path: &str) -> io::Result<String> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer).await?;
        Ok(answer)
    }

    #[tokio::test]
    async fn the_page_server_lives_as_long_as_its_window() {
        let bound = Bound::new(Source::Fixture(Box::new(office())))
            .await
            .expect("binds");
        let link = bound.link("/n/home?need=x", Some("mfrggzdf"));
        let port = bound.port;
        assert!(link.starts_with(&format!("http://127.0.0.1:{port}/n/home?need=x#t=")));
        assert!(link.ends_with("&join=mfrggzdf"));

        let page = bound.serve();
        let answer = get(port, "/n/home").await.expect("serves the page");
        assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
        assert!(answer.contains("content-security-policy"));

        drop(page);
        let mut refused = false;
        for _ in 0..50 {
            if get(port, "/").await.is_err() {
                refused = true;
                break;
            }
            tokio::time::sleep(core::time::Duration::from_millis(20)).await;
        }
        assert!(refused, "closing the window stops its server");
    }

    #[test]
    fn the_web_view_stays_on_its_own_server() {
        let url = |text: &str| tauri::Url::parse(text).expect("a URL");
        assert!(stays_inside(&url("http://127.0.0.1:5000/n/home"), 5000));
        assert!(!stays_inside(&url("http://localhost:5000/"), 5000));
        assert!(!stays_inside(&url("http://[::1]:5000/"), 5000));
        assert!(!stays_inside(&url("http://user:pw@127.0.0.1:5000/"), 5000));
        assert!(!stays_inside(&url("https://127.0.0.1:5000/"), 5000));
        assert!(!stays_inside(&url("http://127.0.0.1:5001/"), 5000));
        assert!(!stays_inside(
            &url("https://apps.apple.com/app/wireguard"),
            5000
        ));
        assert!(!stays_inside(&url("kith://join/abc"), 5000));
    }
}
