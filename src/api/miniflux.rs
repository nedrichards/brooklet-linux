use async_trait::async_trait;
use reqwest::{Client, Method, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    error::BrookletError,
    model::{EntryId, FailureKind, classify_http_status},
    services::{
        traits::MinifluxApi,
        url_policy::{service_url, service_url_with_policy},
    },
};

pub const MINIMUM_MINIFLUX_VERSION: &str = "2.3.2";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct UserDto {
    pub id: i64,
    pub username: String,
    #[serde(default)]
    pub is_admin: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct VersionDto {
    pub version: String,
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ServerIdentity {
    pub user: UserDto,
    pub version: VersionDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct CategoryDto {
    pub id: i64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct FeedDto {
    pub id: i64,
    #[serde(default)]
    pub category: Option<CategoryDto>,
    pub title: String,
    #[serde(default)]
    pub site_url: String,
    #[serde(default)]
    pub feed_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct EntryDto {
    pub id: EntryId,
    pub feed_id: i64,
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub author: Option<String>,
    pub published_at: String,
    pub changed_at: String,
    #[serde(default)]
    pub content: String,
    pub status: String,
    pub starred: bool,
    #[serde(default)]
    pub reading_time: u32,
    #[serde(default)]
    pub feed: Option<FeedDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct EntryIdsDto {
    pub total: usize,
    pub entry_ids: Vec<EntryId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct EntriesDto {
    pub total: usize,
    pub entries: Vec<EntryDto>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryStatus {
    Unread,
    Read,
}

impl EntryStatus {
    fn wire_value(self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Read => "read",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryQuery {
    pub changed_before: Option<i64>,
    pub after_entry_id: Option<i64>,
    pub status: Option<EntryStatus>,
    pub changed_after: Option<i64>,
    pub direction: &'static str,
    pub order: &'static str,
    pub limit: usize,
    pub offset: usize,
}

impl Default for EntryQuery {
    fn default() -> Self {
        Self {
            status: None,
            changed_after: None,
            changed_before: None,
            after_entry_id: None,
            direction: "desc",
            order: "changed_at",
            limit: 100,
            offset: 0,
        }
    }
}

#[derive(Serialize)]
struct StatusMutation<'a> {
    entry_ids: &'a [EntryId],
    status: &'static str,
}

#[derive(Serialize)]
struct StarMutation<'a> {
    entry_ids: &'a [EntryId],
    starred: bool,
}

#[derive(Serialize)]
struct Subscription<'a> {
    feed_url: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    category_id: Option<i64>,
}

pub struct ReqwestMinifluxApi {
    base: Url,
    token: String,
    client: Client,
}

impl ReqwestMinifluxApi {
    pub fn new(server_url: &str, token: String) -> Result<Self, BrookletError> {
        Self::with_url_policy(server_url, token, false)
    }

    fn with_url_policy(
        server_url: &str,
        token: String,
        allow_http: bool,
    ) -> Result<Self, BrookletError> {
        let mut base = if allow_http {
            service_url_with_policy(server_url, true)?
        } else {
            service_url(server_url)?
        };
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        let client = Client::builder()
            .redirect(Policy::none())
            .https_only(!allow_http)
            .build()
            .map_err(transport_error)?;
        Ok(Self {
            base,
            token,
            client,
        })
    }

    #[cfg(test)]
    fn for_test(server_url: &str, token: &str) -> Self {
        Self::with_url_policy(server_url, token.to_owned(), true).unwrap()
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base
            .join(path)
            .expect("static Miniflux endpoint must be valid")
    }

    fn request(&self, method: Method, url: Url) -> reqwest::RequestBuilder {
        self.client
            .request(method, url)
            .header("X-Auth-Token", &self.token)
            .header(reqwest::header::ACCEPT, "application/json")
    }

    async fn send_json<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, BrookletError> {
        let response = request.send().await.map_err(transport_error)?;
        ensure_success(response.status())?;
        response.json().await.map_err(transport_error)
    }

    async fn send_empty(&self, request: reqwest::RequestBuilder) -> Result<(), BrookletError> {
        let response = request.send().await.map_err(transport_error)?;
        ensure_success(response.status())
    }
}

#[async_trait]
impl MinifluxApi for ReqwestMinifluxApi {
    async fn validate(&self) -> Result<ServerIdentity, BrookletError> {
        let user = self
            .send_json(self.request(Method::GET, self.endpoint("v1/me")))
            .await?;
        let version: VersionDto = self
            .send_json(self.request(Method::GET, self.endpoint("v1/version")))
            .await?;
        if version_number(&version.version) < version_number(MINIMUM_MINIFLUX_VERSION) {
            return Err(BrookletError::UnsupportedServer {
                found: version.version,
                required: MINIMUM_MINIFLUX_VERSION,
            });
        }
        Ok(ServerIdentity { user, version })
    }

    async fn categories(&self) -> Result<Vec<CategoryDto>, BrookletError> {
        self.send_json(self.request(Method::GET, self.endpoint("v1/categories")))
            .await
    }

    async fn feeds(&self) -> Result<Vec<FeedDto>, BrookletError> {
        self.send_json(self.request(Method::GET, self.endpoint("v1/feeds")))
            .await
    }

    async fn entries(&self, query: &EntryQuery) -> Result<EntriesDto, BrookletError> {
        let mut url = self.endpoint("v1/entries");
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(status) = query.status {
                pairs.append_pair("status", status.wire_value());
            }
            if let Some(changed_after) = query.changed_after {
                pairs.append_pair("changed_after", &changed_after.to_string());
            }
            if let Some(value) = query.changed_before {
                pairs.append_pair("changed_before", &value.to_string());
            }
            if let Some(value) = query.after_entry_id {
                pairs.append_pair("after_entry_id", &value.to_string());
            }
            pairs
                .append_pair("direction", query.direction)
                .append_pair("order", query.order)
                .append_pair("limit", &query.limit.to_string())
                .append_pair("offset", &query.offset.to_string());
        }
        self.send_json(self.request(Method::GET, url)).await
    }

    async fn entry_ids(&self, limit: usize, offset: usize) -> Result<EntryIdsDto, BrookletError> {
        let mut url = self.endpoint("v1/entries/ids");
        url.query_pairs_mut()
            .append_pair("limit", &limit.to_string())
            .append_pair("offset", &offset.to_string());
        self.send_json(self.request(Method::GET, url)).await
    }

    async fn entry(&self, entry_id: EntryId) -> Result<EntryDto, BrookletError> {
        if entry_id <= 0 {
            return Err(BrookletError::InvalidServiceUrl(
                "entry ID must be positive".into(),
            ));
        }
        self.send_json(self.request(
            Method::GET,
            self.endpoint(&format!("v1/entries/{entry_id}")),
        ))
        .await
    }

    async fn set_read(&self, entry_ids: &[EntryId], read: bool) -> Result<(), BrookletError> {
        self.send_empty(self.request(Method::PUT, self.endpoint("v1/entries")).json(
            &StatusMutation {
                entry_ids,
                status: if read { "read" } else { "unread" },
            },
        ))
        .await
    }

    async fn set_starred(&self, entry_ids: &[EntryId], starred: bool) -> Result<(), BrookletError> {
        self.send_empty(
            self.request(Method::PUT, self.endpoint("v1/entries"))
                .json(&StarMutation { entry_ids, starred }),
        )
        .await
    }

    async fn save_to_integration(&self, entry_id: EntryId) -> Result<(), BrookletError> {
        self.send_empty(self.request(
            Method::POST,
            self.endpoint(&format!("v1/entries/{entry_id}/save")),
        ))
        .await
    }

    async fn refresh_feeds(&self) -> Result<(), BrookletError> {
        self.send_empty(self.request(Method::PUT, self.endpoint("v1/feeds/refresh")))
            .await
    }

    async fn subscribe(
        &self,
        feed_url: &str,
        category_id: Option<i64>,
    ) -> Result<i64, BrookletError> {
        // Creation returns an ID, not the full feed returned by GET /v1/feeds.
        // Do not turn a successful creation into a failure by fetching metadata
        // here: the UI follows creation with the normal sync path.
        #[derive(Deserialize)]
        struct CreatedFeed {
            feed_id: i64,
        }
        let created: CreatedFeed =
            self.send_json(self.request(Method::POST, self.endpoint("v1/feeds")).json(
                &Subscription {
                    feed_url,
                    category_id,
                },
            ))
            .await?;
        Ok(created.feed_id)
    }
}

fn ensure_success(status: StatusCode) -> Result<(), BrookletError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(BrookletError::Http {
            status: status.as_u16(),
            kind: classify_http_status(status.as_u16()),
        })
    }
}

fn transport_error(source: reqwest::Error) -> BrookletError {
    let detail = source.to_string().to_ascii_lowercase();
    let kind = if detail.contains("certificate") || detail.contains("tls") || detail.contains("ssl")
    {
        FailureKind::Certificate
    } else {
        FailureKind::Retryable
    };
    BrookletError::Transport { kind, source }
}

fn version_number(value: &str) -> (u64, u64, u64) {
    let mut parts = value.split('-').next().unwrap_or_default().split('.');
    (
        parts.next().and_then(|part| part.parse().ok()).unwrap_or(0),
        parts.next().and_then(|part| part.parse().ok()).unwrap_or(0),
        parts.next().and_then(|part| part.parse().ok()).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use super::*;

    #[tokio::test]
    async fn validation_uses_both_endpoints_and_accepts_2_3_2() {
        let (base, requests) = serve(vec![
            response(200, r#"{"id":1,"username":"ned","future":true}"#, &[]),
            response(200, r#"{"version":"2.3.2"}"#, &[]),
        ]);
        let identity = ReqwestMinifluxApi::for_test(&base, "secret")
            .validate()
            .await
            .unwrap();
        assert_eq!(identity.user.username, "ned");
        let first = requests.recv().unwrap();
        let second = requests.recv().unwrap();
        assert!(first.starts_with("GET /v1/me HTTP/1.1"));
        assert!(first.to_ascii_lowercase().contains("x-auth-token: secret"));
        assert!(second.starts_with("GET /v1/version HTTP/1.1"));
    }

    #[tokio::test]
    async fn entries_and_mutations_match_miniflux_contracts() {
        let (base, requests) = serve(vec![
            response(200, r#"{"total":0,"entries":[]}"#, &[]),
            response(204, "", &[]),
            response(204, "", &[]),
        ]);
        let api = ReqwestMinifluxApi::for_test(&base, "secret");
        api.entries(&EntryQuery {
            status: Some(EntryStatus::Unread),
            order: "published_at",
            ..EntryQuery::default()
        })
        .await
        .unwrap();
        api.set_read(&[4, 9], true).await.unwrap();
        api.set_starred(&[9], true).await.unwrap();
        let entries = requests.recv().unwrap();
        assert!(entries.starts_with("GET /v1/entries?status=unread&direction=desc&order=published_at&limit=100&offset=0 HTTP/1.1"));
        assert!(
            requests
                .recv()
                .unwrap()
                .contains(r#"{"entry_ids":[4,9],"status":"read"}"#)
        );
        assert!(
            requests
                .recv()
                .unwrap()
                .contains(r#"{"entry_ids":[9],"starred":true}"#)
        );
    }

    #[tokio::test]
    async fn subscription_accepts_creation_id_without_requesting_metadata() {
        for category_id in [None, Some(22)] {
            let (base, requests) = serve(vec![response(201, r#"{"feed_id":262}"#, &[])]);
            let api = ReqwestMinifluxApi::for_test(&base, "secret");
            assert_eq!(
                api.subscribe("https://example.org/feed.atom", category_id)
                    .await
                    .unwrap(),
                262
            );
            let request = requests.recv().unwrap();
            assert!(request.starts_with("POST /v1/feeds HTTP/1.1"));
            let payload: serde_json::Value =
                serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(payload["feed_url"], "https://example.org/feed.atom");
            if let Some(id) = category_id {
                assert_eq!(payload["category_id"], id);
            } else {
                assert!(payload.get("category_id").is_none());
            }
            assert!(requests.recv().is_err());
        }
    }

    #[tokio::test]
    async fn subscription_reports_server_rejection_and_invalid_response() {
        for (status, body) in [(400, r#"{"error_message":"invalid feed"}"#), (201, "{}")] {
            let (base, requests) = serve(vec![response(status, body, &[])]);
            let result = ReqwestMinifluxApi::for_test(&base, "secret")
                .subscribe("https://example.org/feed.atom", None)
                .await;
            if status == 400 {
                assert!(matches!(
                    result,
                    Err(BrookletError::Http { status: 400, .. })
                ));
            } else {
                assert!(result.is_err());
            }
            assert!(
                requests
                    .recv()
                    .unwrap()
                    .starts_with("POST /v1/feeds HTTP/1.1")
            );
            assert!(requests.recv().is_err());
        }
    }

    #[tokio::test]
    async fn credential_requests_do_not_follow_redirects() {
        let (base, requests) = serve(vec![response(302, "", &[("Location", "/elsewhere")])]);
        let error = ReqwestMinifluxApi::for_test(&base, "secret")
            .categories()
            .await
            .unwrap_err();
        assert!(matches!(error, BrookletError::Http { status: 302, .. }));
        assert!(
            requests
                .recv()
                .unwrap()
                .starts_with("GET /v1/categories HTTP/1.1")
        );
    }

    #[tokio::test]
    async fn reconciliation_queries_use_id_filters_and_unfiltered_inventory() {
        let (base, requests) = serve(vec![
            response(200, r#"{"total":0,"entries":[]}"#, &[]),
            response(200, r#"{"total":2,"entry_ids":[99,42]}"#, &[]),
        ]);
        let api = ReqwestMinifluxApi::for_test(&base, "secret");
        api.entries(&EntryQuery {
            changed_after: Some(940),
            changed_before: Some(1001),
            after_entry_id: Some(42),
            order: "id",
            direction: "asc",
            ..EntryQuery::default()
        })
        .await
        .unwrap();
        let request = requests.recv().unwrap();
        assert!(request.contains("changed_after=940&changed_before=1001&after_entry_id=42&direction=asc&order=id&limit=100&offset=0"));
        let ids = api.entry_ids(10_000, 10_000).await.unwrap();
        assert_eq!(ids.entry_ids, [99, 42]);
        let request = requests.recv().unwrap();
        assert!(request.starts_with("GET /v1/entries/ids?limit=10000&offset=10000 HTTP/1.1"));
        assert!(!request.contains("status="));
    }

    #[test]
    fn semantic_version_comparison_handles_suffixes() {
        assert_eq!(version_number("2.3.2"), (2, 3, 2));
        assert!(version_number("2.4.0-alpha") > version_number(MINIMUM_MINIFLUX_VERSION));
        assert!(version_number("2.3.1") < version_number(MINIMUM_MINIFLUX_VERSION));
    }

    fn serve(responses: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(header_end) =
                        request.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        let headers = String::from_utf8_lossy(&request[..header_end + 4]);
                        let content_length = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|value| value.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if request.len() >= header_end + 4 + content_length {
                            break;
                        }
                    }
                }
                sender
                    .send(String::from_utf8_lossy(&request).into_owned())
                    .unwrap();
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (format!("http://{address}"), receiver)
    }

    fn response(status: u16, body: &str, headers: &[(&str, &str)]) -> String {
        let reason = match status {
            200 => "OK",
            204 => "No Content",
            302 => "Found",
            _ => "Error",
        };
        let extra = headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect::<String>();
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }
}
