use async_trait::async_trait;
use reqwest::{Client, redirect::Policy};
use serde::Serialize;

use crate::{
    error::BrookletError,
    model::{FailureKind, classify_http_status},
    services::{
        traits::KarakeepApi,
        url_policy::{service_url, service_url_with_policy},
    },
};

pub struct ReqwestKarakeepApi {
    endpoint: url::Url,
    key: String,
    client: Client,
}

#[derive(Serialize)]
struct Bookmark<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    url: &'a str,
    title: &'a str,
}

impl ReqwestKarakeepApi {
    pub fn new(endpoint: &str, key: String) -> Result<Self, BrookletError> {
        Self::with_policy(endpoint, key, false)
    }

    fn with_policy(
        endpoint: &str,
        key: String,
        allow_http_for_tests: bool,
    ) -> Result<Self, BrookletError> {
        let endpoint = if allow_http_for_tests {
            service_url_with_policy(endpoint, true)?
        } else {
            service_url(endpoint)?
        };
        let client = Client::builder()
            .https_only(!allow_http_for_tests)
            .redirect(Policy::none())
            .build()
            .map_err(|source| BrookletError::Transport {
                kind: FailureKind::Retryable,
                source,
            })?;
        Ok(Self {
            endpoint,
            key,
            client,
        })
    }
}

#[async_trait]
impl KarakeepApi for ReqwestKarakeepApi {
    async fn validate(&self) -> Result<(), BrookletError> {
        // Read-only authentication and endpoint check; never create a test bookmark.
        let response = self
            .client
            .get(self.endpoint.clone())
            .timeout(std::time::Duration::from_secs(30))
            .query(&[("limit", "1"), ("includeContent", "false")])
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(|source| BrookletError::Transport {
                kind: FailureKind::Retryable,
                source,
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(BrookletError::Http {
                status: status.as_u16(),
                kind: classify_http_status(status.as_u16()),
            });
        }
        #[derive(serde::Deserialize)]
        struct Page {
            bookmarks: Vec<serde_json::Value>,
        }
        let page = response
            .json::<Page>()
            .await
            .map_err(|source| BrookletError::Transport {
                kind: FailureKind::MalformedRequest,
                source,
            })?;
        let _ = page.bookmarks;
        Ok(())
    }

    async fn save(&self, canonical_url: &str, title: &str) -> Result<(), BrookletError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.key)
            .json(&Bookmark {
                kind: "link",
                url: canonical_url,
                title,
            })
            .send()
            .await
            .map_err(|source| BrookletError::Transport {
                kind: FailureKind::Retryable,
                source,
            })?;
        let status = response.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(BrookletError::Http {
                status: status.as_u16(),
                kind: classify_http_status(status.as_u16()),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    fn serve(status: u16) -> (String, mpsc::Receiver<String>) {
        serve_body(status, "")
    }

    fn serve_body(status: u16, body: &str) -> (String, mpsc::Receiver<String>) {
        let body = body.to_owned();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0_u8; 4096];
                let count = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..count]);
                if let Some(header_end) =
                    request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..header_end + 4]);
                    let size = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap_or(0);
                    if request.len() >= header_end + 4 + size {
                        break;
                    }
                }
            }
            sender
                .send(String::from_utf8_lossy(&request).into_owned())
                .unwrap();
            let reply = format!(
                "HTTP/1.1 {status} Result\r\nLocation: /redirected\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (format!("http://{address}/api/v1/bookmarks"), receiver)
    }

    #[tokio::test]
    async fn direct_delivery_uses_bearer_json_and_accepts_existing_bookmark() {
        let (endpoint, requests) = serve(200);
        let api = ReqwestKarakeepApi::with_policy(&endpoint, "secret".into(), true).unwrap();
        api.save("https://example.com/article", "Article")
            .await
            .unwrap();
        let request = requests.recv().unwrap();
        assert!(request.starts_with("POST /api/v1/bookmarks HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer secret")
        );
        assert!(request.contains("\"url\":\"https://example.com/article\""));
        assert!(!request.contains("\"source\""));
    }

    #[tokio::test]
    async fn direct_delivery_does_not_follow_credential_redirects() {
        let (endpoint, requests) = serve(302);
        let api = ReqwestKarakeepApi::with_policy(&endpoint, "secret".into(), true).unwrap();
        assert!(matches!(
            api.save("https://example.com", "Example").await,
            Err(BrookletError::Http { status: 302, .. })
        ));
        assert_eq!(requests.recv().unwrap().matches("POST ").count(), 1);
    }
    #[tokio::test]
    async fn settings_validation_is_read_only_and_checks_bookmarks_response() {
        let (endpoint, requests) = serve_body(200, r#"{"bookmarks":[],"nextCursor":null}"#);
        let api = ReqwestKarakeepApi::with_policy(&endpoint, "secret".into(), true).unwrap();
        api.validate().await.unwrap();
        let request = requests.recv().unwrap();
        assert!(request.starts_with("GET /api/v1/bookmarks?limit=1&includeContent=false HTTP/1.1"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer secret")
        );
        for (status, body) in [
            (401, ""),
            (302, ""),
            (200, "<html>login</html>"),
            (200, "{}"),
            (200, r#"{"bookmarks":null}"#),
        ] {
            let (endpoint, requests) = serve_body(status, body);
            let api = ReqwestKarakeepApi::with_policy(&endpoint, "secret".into(), true).unwrap();
            assert!(api.validate().await.is_err());
            assert!(requests.recv().unwrap().starts_with("GET "));
        }
    }
    #[tokio::test]
    async fn arbitrary_conflicts_are_not_mistaken_for_saved_bookmarks() {
        let (endpoint, requests) = serve(409);
        let api = ReqwestKarakeepApi::with_policy(&endpoint, "secret".into(), true).unwrap();
        assert!(matches!(
            api.save("https://example.com", "Article").await,
            Err(BrookletError::Http { status: 409, .. })
        ));
        assert!(requests.recv().unwrap().starts_with("POST "));
    }
}
