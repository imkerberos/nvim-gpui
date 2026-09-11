use futures::future::BoxFuture;
use gpui::http_client::{
    anyhow, http::HeaderValue, AsyncBody, HttpClient, Request, Response, Result as HttpResult, Uri,
    Url,
};
use std::{
    any::type_name,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub(crate) const REPOSITORY: &str = "imkerberos/nvim-gpui";
const UPDATE_CHECK_ENDPOINT_ENV: &str = "NVIM_GPUI_UPDATE_CHECK_ENDPOINT";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Status {
    #[default]
    NeverChecked,
    Checking,
    UpToDate,
    Available {
        version: String,
        url: String,
    },
    Failed(String),
}

pub(crate) fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn is_due(last_checked: u64) -> bool {
    last_checked == 0 || unix_timestamp().saturating_sub(last_checked) >= CHECK_INTERVAL.as_secs()
}

pub(crate) async fn check_latest(http: Arc<dyn HttpClient>) -> Result<Status, String> {
    let endpoint = configured_endpoint();
    check_latest_with_endpoint(http, endpoint.as_deref()).await
}

async fn check_latest_with_endpoint(
    http: Arc<dyn HttpClient>,
    endpoint: Option<&str>,
) -> Result<Status, String> {
    let http = match endpoint {
        Some(endpoint) => endpoint_http_client(http, endpoint)?,
        None => http,
    };

    let release = gpui::http_client::github::latest_github_release(REPOSITORY, true, false, http)
        .await
        .map_err(|error| error.to_string())?;

    status_for_tag(&release.tag_name)
}

fn configured_endpoint() -> Option<String> {
    std::env::var(UPDATE_CHECK_ENDPOINT_ENV)
        .ok()
        .map(|endpoint| endpoint.trim().to_owned())
        .filter(|endpoint| !endpoint.is_empty())
}

fn endpoint_http_client(
    inner: Arc<dyn HttpClient>,
    endpoint: &str,
) -> Result<Arc<dyn HttpClient>, String> {
    let endpoint = endpoint.parse::<Uri>().map_err(|error| {
        format!("invalid {UPDATE_CHECK_ENDPOINT_ENV} value {endpoint:?}: {error}")
    })?;
    log::debug!(
        target: "nvim_gpui::update_check",
        "using update-check endpoint override"
    );
    Ok(Arc::new(EndpointHttpClient { inner, endpoint }))
}

struct EndpointHttpClient {
    inner: Arc<dyn HttpClient>,
    endpoint: Uri,
}

impl HttpClient for EndpointHttpClient {
    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        self.inner.user_agent()
    }

    fn send(
        &self,
        mut request: Request<AsyncBody>,
    ) -> BoxFuture<'static, HttpResult<Response<AsyncBody>>> {
        *request.uri_mut() = self.endpoint.clone();
        self.inner.send(request)
    }

    fn proxy(&self) -> Option<&Url> {
        self.inner.proxy()
    }
}

fn status_for_tag(tag: &str) -> Result<Status, String> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let latest = semver::Version::parse(version)
        .map_err(|error| format!("GitHub returned invalid release tag {tag:?}: {error}"))?;
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("CARGO_PKG_VERSION must be valid SemVer");

    if latest > current {
        Ok(Status::Available {
            version: version.to_owned(),
            url: release_url(tag),
        })
    } else {
        Ok(Status::UpToDate)
    }
}

fn release_url(tag: &str) -> String {
    let mut url = Url::parse(&format!("https://github.com/{REPOSITORY}/releases/tag"))
        .expect("release URL base must be valid");
    url.path_segments_mut()
        .expect("GitHub release URL must support path segments")
        .push(tag);
    url.to_string()
}

pub(crate) fn http_client() -> Arc<dyn HttpClient> {
    Arc::new(ReqwestHttpClient {
        client: reqwest::blocking::Client::builder()
            .user_agent(concat!("nvim-gpui/", env!("CARGO_PKG_VERSION")))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("nvim-gpui update-check HTTP client must initialize"),
        user_agent: HeaderValue::from_static(concat!("nvim-gpui/", env!("CARGO_PKG_VERSION"))),
    })
}

struct ReqwestHttpClient {
    client: reqwest::blocking::Client,
    user_agent: HeaderValue,
}

impl HttpClient for ReqwestHttpClient {
    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> BoxFuture<'static, HttpResult<Response<AsyncBody>>> {
        let (parts, _body) = request.into_parts();
        let method = match reqwest::Method::from_bytes(parts.method.as_str().as_bytes()) {
            Ok(method) => method,
            Err(error) => return Box::pin(async move { Err(error.into()) }),
        };
        let uri = parts.uri.to_string();
        let headers = parts.headers;
        let client = self.client.clone();
        let (sender, receiver) = async_channel::bounded(1);

        if let Err(error) = std::thread::Builder::new()
            .name("nvim-gpui-update-check".to_owned())
            .spawn(move || {
                let result = client
                    .request(method, uri)
                    .headers(headers)
                    .send()
                    .and_then(|response| {
                        let status = response.status().as_u16();
                        let body = response.bytes()?;
                        Ok((status, body))
                    });
                let _ = sender.send_blocking(result);
            })
        {
            return Box::pin(async move { Err(error.into()) });
        }

        Box::pin(async move {
            let (status, body) = receiver
                .recv()
                .await
                .map_err(|error| anyhow!("update check worker stopped: {error}"))??;

            Ok(Response::builder()
                .status(status)
                .body(AsyncBody::from(body.to_vec()))?)
        })
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        check_latest_with_endpoint, http_client, is_due, release_url, status_for_tag, Status,
    };
    use gpui::http_client::{AsyncBody, Request};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    fn newer_release_tag() -> String {
        let mut version = semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .expect("package version should be valid SemVer");
        version.patch += 1;
        format!("v{version}")
    }

    #[test]
    fn detects_newer_stable_release() {
        let tag = newer_release_tag();
        let version = tag
            .strip_prefix('v')
            .expect("test tag should have a v prefix");
        assert_eq!(
            status_for_tag(&tag).expect("version should parse"),
            Status::Available {
                version: version.to_owned(),
                url: format!("https://github.com/imkerberos/nvim-gpui/releases/tag/{tag}"),
            }
        );
    }

    #[test]
    fn ignores_current_or_older_release() {
        assert_eq!(status_for_tag("v0.7.1").unwrap(), Status::UpToDate);
        assert_eq!(status_for_tag("v0.7.0").unwrap(), Status::UpToDate);
    }

    #[test]
    fn rejects_invalid_release_tag() {
        let error = status_for_tag("nightly").expect_err("invalid tag should fail");
        assert!(error.contains("invalid release tag"));
    }

    #[test]
    fn checks_at_most_once_per_day() {
        assert!(is_due(0));
        assert!(!is_due(super::unix_timestamp()));
    }

    #[test]
    fn release_url_escapes_tag_path_segments() {
        assert_eq!(
            release_url("release/0.7.2"),
            "https://github.com/imkerberos/nvim-gpui/releases/tag/release%2F0.7.2"
        );
    }

    #[test]
    fn blocking_http_client_works_without_a_tokio_context() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should have an address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut request = [0; 1024];
            let bytes_read = stream
                .read(&mut request)
                .expect("test server should read the request");
            assert!(bytes_read > 0, "test server should receive request data");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .expect("test server should write the response");
        });
        let request = Request::builder()
            .uri(format!("http://localhost:{}/", address.port()))
            .body(AsyncBody::empty())
            .expect("test request should build");

        let response = futures_lite::future::block_on(http_client().send(request))
            .expect("blocking HTTP client should return a response");
        assert_eq!(response.status().as_u16(), 200);
        server.join().expect("test server should finish");
    }

    #[test]
    fn endpoint_override_detects_newer_release() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test listener should bind");
        let address = listener
            .local_addr()
            .expect("test listener should have an address");
        let tag = newer_release_tag();
        let version = tag
            .strip_prefix('v')
            .expect("test tag should have a v prefix");
        let body = format!(
            r#"[{{"tag_name":"{tag}","prerelease":false,"assets":[{{"name":"test","browser_download_url":"https://example.com/test","digest":null}}],"tarball_url":"https://example.com/tarball","zipball_url":"https://example.com/zip"}}]"#
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut request = [0; 1024];
            let bytes_read = stream
                .read(&mut request)
                .expect("test server should read the request");
            let request = String::from_utf8_lossy(&request[..bytes_read]);
            assert!(request.starts_with("GET /fake/releases HTTP/1.1"));
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should write the response");
        });
        let endpoint = format!("http://localhost:{}/fake/releases", address.port());

        let status = futures_lite::future::block_on(check_latest_with_endpoint(
            http_client(),
            Some(&endpoint),
        ))
        .expect("endpoint override should return a status");

        assert_eq!(
            status,
            Status::Available {
                version: version.to_owned(),
                url: format!("https://github.com/imkerberos/nvim-gpui/releases/tag/{tag}"),
            }
        );
        server.join().expect("test server should finish");
    }
}
