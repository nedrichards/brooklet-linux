use async_trait::async_trait;
use reqwest::{Client, StatusCode, redirect::Policy};
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
    source: &'static str,
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
    async fn save(&self, canonical_url: &str, title: &str) -> Result<(), BrookletError> {
        let response = self
            .client
            .post(self.endpoint.clone())
            .bearer_auth(&self.key)
            .json(&Bookmark {
                kind: "link",
                url: canonical_url,
                title,
                source: "brooklet-linux",
            })
            .send()
            .await
            .map_err(|source| BrookletError::Transport {
                kind: FailureKind::Retryable,
                source,
            })?;
        let status = response.status();
        if status.is_success() || status == StatusCode::CONFLICT {
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
                "HTTP/1.1 {status} Result\r\nLocation: /redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        (format!("http://{address}/api/v1/bookmarks"), receiver)
    }

    #[tokio::test]
    async fn direct_delivery_uses_bearer_json_and_accepts_existing_bookmark() {
        let (endpoint, requests) = serve(409);
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
        assert!(request.contains("\"source\":\"brooklet-linux\""));
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
}
