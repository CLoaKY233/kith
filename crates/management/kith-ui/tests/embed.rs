//! Runs the built `kith-ui` against the office fixture and checks what `build.rs` embedded:
//! the built page with every file it loads, or `fallback.html` when the page was not built.
//!
//! Set `KITH_UI_EXPECT_PAGE=1` to fail when the fallback was embedded; CI's `ui` job does,
//! after building the page.

#[cfg(test)]
#[expect(
    clippy::panic_in_result_fn,
    reason = "assertions fail the test; ? keeps the setup short"
)]
mod tests {
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpStream;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// The policy the spec (§2.3) puts on every answer.
    const CSP: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; frame-ancestors 'none'";
    /// What the server fills the markers with; none may be left in a served page.
    const MARKER: &str = "<!--kith:";

    fn crate_dir() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    /// `kith-ui --fixture`, stopped when dropped.
    struct Server {
        child: Child,
        port: u16,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            let _gone = self.child.kill();
            let _reaped = self.child.wait();
        }
    }

    impl Server {
        /// Starts the binary on a free port and waits for the link it prints.
        fn start() -> TestResult<Self> {
            let fixture: PathBuf = crate_dir().join("web/fixtures/office.json");
            let mut child = Command::new(env!("CARGO_BIN_EXE_kith-ui"))
                .arg("--fixture")
                .arg(&fixture)
                .args(["--no-open", "--port", "0"])
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let mut server = Self { child, port: 0 };
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line)?;
            let port = line
                .split("http://127.0.0.1:")
                .nth(1)
                .and_then(|rest| rest.split('/').next())
                .ok_or_else(|| format!("no link in {line:?}"))?;
            server.port = port.parse()?;
            Ok(server)
        }

        /// One GET over HTTP/1.1, as the browser on this machine would send it.
        fn get(&self, path: &str, headers: &[&str]) -> TestResult<Answer> {
            let mut stream = TcpStream::connect(("127.0.0.1", self.port))?;
            let mut request = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n",
                self.port
            );
            for header in headers {
                request.push_str(header);
                request.push_str("\r\n");
            }
            request.push_str("\r\n");
            stream.write_all(request.as_bytes())?;
            let mut raw = Vec::new();
            stream.read_to_end(&mut raw)?;
            Answer::parse(&raw)
        }
    }

    /// A parsed HTTP answer.
    struct Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Answer {
        fn parse(raw: &[u8]) -> TestResult<Self> {
            let split = raw
                .windows(4)
                .position(|window| window == b"\r\n\r\n")
                .ok_or("no end of headers")?;
            let head = std::str::from_utf8(raw.get(..split).ok_or("short answer")?)?;
            let body = raw.get(split + 4..).unwrap_or_default().to_vec();
            let mut lines = head.lines();
            let status = lines
                .next()
                .and_then(|line| line.split(' ').nth(1))
                .ok_or("no status line")?
                .parse()?;
            let headers = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
                .collect();
            Ok(Self {
                status,
                headers,
                body,
            })
        }

        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }

        fn text(&self) -> TestResult<&str> {
            Ok(std::str::from_utf8(&self.body)?)
        }
    }

    /// Root-relative paths a page loads, as `build.rs` finds them.
    fn references(html: &str) -> Vec<&str> {
        ["src=\"/", "href=\"/"]
            .iter()
            .flat_map(|attribute| html.split(attribute).skip(1))
            .filter_map(|rest| rest.split('"').next())
            .filter_map(|path| path.split(['?', '#']).next())
            .filter(|path| !path.is_empty() && !path.starts_with('/'))
            .collect()
    }

    fn expected_type(path: &str) -> &'static str {
        match path.rsplit_once('.').map(|(_, extension)| extension) {
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("woff2") => "font/woff2",
            Some("svg") => "image/svg+xml",
            _ => "application/octet-stream",
        }
    }

    #[test]
    fn the_served_page_is_complete_and_signed_out_without_a_session() -> TestResult {
        let server = Server::start()?;
        let page = server.get("/", &[])?;
        assert_eq!(page.status, 200);
        assert_eq!(page.header("content-security-policy"), Some(CSP));
        assert_eq!(page.header("cache-control"), Some("no-store"));
        let html = page.text()?;
        assert!(!html.contains(MARKER), "every marker is filled: {html}");
        assert!(html.contains(r#"<script type="application/json" id="boot">null</script>"#));
        assert!(html.contains("<h1>This page needs a fresh link.</h1>"));

        let assets = references(html);
        let built = !assets.is_empty();
        if std::env::var_os("KITH_UI_EXPECT_PAGE").is_some() {
            assert!(
                built,
                "kith-ui embedded fallback.html; build web/dist first"
            );
        }
        if !built {
            assert!(html.contains("This kith-ui was built without its page."));
            return Ok(());
        }
        for asset in assets {
            let plain = server.get(&format!("/{asset}"), &[])?;
            assert_eq!(plain.status, 200, "/{asset}");
            assert_eq!(plain.header("content-type"), Some(expected_type(asset)));
            assert_eq!(
                plain.header("cache-control"),
                Some("public, max-age=31536000, immutable"),
                "/{asset} is hashed, so it is cached for good"
            );
            let on_disk = std::fs::read(crate_dir().join("web/dist").join(asset))?;
            assert_eq!(plain.body, on_disk, "/{asset} is the file in web/dist");
            let text = matches!(
                asset.rsplit_once('.').map(|(_, extension)| extension),
                Some("js" | "css")
            );
            if text {
                let packed = server.get(&format!("/{asset}"), &["Accept-Encoding: br"])?;
                assert_eq!(packed.header("content-encoding"), Some("br"), "/{asset}");
                assert!(packed.body.len() < on_disk.len());
            }
        }
        Ok(())
    }

    #[test]
    fn only_the_spec_routes_serve_the_page() -> TestResult {
        let server = Server::start()?;
        for route in ["/welcome", "/this", "/n/home", "/n/home/devices/nas"] {
            let page = server.get(route, &[])?;
            assert_eq!(page.status, 200, "{route}");
            assert_eq!(
                page.header("content-type"),
                Some("text/html; charset=utf-8")
            );
        }
        for other in ["/n/home/inbox", "/index.html", "/admin", "/fallback.html"] {
            assert_eq!(server.get(other, &[])?.status, 404, "{other}");
        }
        Ok(())
    }

    /// The fallback must work without JavaScript under the CSP, and keep both markers so the
    /// server can fill them.
    #[test]
    fn the_fallback_page_works_under_the_policy() -> TestResult {
        let html = std::fs::read_to_string(crate_dir().join("fallback.html"))?;
        for marker in ["<!--kith:boot-->", "<!--kith:signed-out-->"] {
            assert_eq!(html.matches(marker).count(), 1, "{marker}");
        }
        assert!(
            !html.contains("<script"),
            "the policy blocks inline scripts"
        );
        assert!(!html.contains("http"), "the policy blocks other origins");
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("name=\"viewport\""));
        Ok(())
    }

    /// The page's source keeps both markers, or the built page loses its boot snapshot and
    /// its signed-out text.
    #[test]
    fn the_page_source_keeps_the_markers() -> TestResult {
        let html = std::fs::read_to_string(crate_dir().join("web/index.html"))?;
        for marker in ["<!--kith:boot-->", "<!--kith:signed-out-->"] {
            assert_eq!(html.matches(marker).count(), 1, "{marker}");
        }
        Ok(())
    }
}
