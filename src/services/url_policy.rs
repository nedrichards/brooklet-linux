use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

use reqwest::{
    Client,
    dns::{Addrs, Name, Resolve, Resolving},
    redirect::Policy,
};

use url::{Host, Url};

use crate::error::BrookletError;

pub const MAX_ARTICLE_IMAGE_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct PublicImageResolver;

impl Resolve for PublicImageResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let hostname = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::net::lookup_host((hostname.as_str(), 443))
                .await?
                .collect::<Vec<_>>();
            let validated = validate_dns_answers(addresses)?;
            Ok(Box::new(validated.into_iter()) as Addrs)
        })
    }
}

pub fn validate_dns_answers(addresses: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, std::io::Error> {
    if addresses.is_empty() || addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "image host resolves to a non-public address",
        ));
    }
    Ok(addresses)
}

pub fn image_client() -> Result<Client, BrookletError> {
    Client::builder()
        .https_only(true)
        .redirect(Policy::none())
        .no_proxy()
        .dns_resolver(Arc::new(PublicImageResolver))
        .build()
        .map_err(image_transport_error)
}

pub async fn fetch_article_image(
    client: &Client,
    candidate: &str,
) -> Result<Vec<u8>, BrookletError> {
    let url = validate_article_image_url(candidate)?;
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(image_transport_error)?;
    if !response.status().is_success() {
        return Err(BrookletError::Http {
            status: response.status().as_u16(),
            kind: crate::model::classify_http_status(response.status().as_u16()),
        });
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARTICLE_IMAGE_BYTES as u64)
    {
        return Err(BrookletError::InvalidServiceUrl(
            "article image exceeds 20 MiB".into(),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(image_transport_error)? {
        append_image_chunk(&mut bytes, &chunk)?;
    }
    Ok(bytes)
}

fn append_image_chunk(destination: &mut Vec<u8>, chunk: &[u8]) -> Result<(), BrookletError> {
    if destination.len().saturating_add(chunk.len()) > MAX_ARTICLE_IMAGE_BYTES {
        return Err(BrookletError::InvalidServiceUrl(
            "article image exceeds 20 MiB".into(),
        ));
    }
    destination.extend_from_slice(chunk);
    Ok(())
}

fn image_transport_error(source: reqwest::Error) -> BrookletError {
    BrookletError::Transport {
        kind: if source.is_builder() {
            crate::model::FailureKind::MalformedRequest
        } else {
            crate::model::FailureKind::Retryable
        },
        source,
    }
}

pub fn service_url(value: &str) -> Result<Url, BrookletError> {
    service_url_with_policy(value, false)
}

pub(crate) fn service_url_with_policy(
    value: &str,
    allow_http_for_tests: bool,
) -> Result<Url, BrookletError> {
    let trimmed = value.trim().trim_end_matches('/');
    let url = Url::parse(trimmed)
        .map_err(|_| BrookletError::InvalidServiceUrl("URL is not valid".into()))?;
    if url.scheme() != "https" && !(allow_http_for_tests && url.scheme() == "http") {
        return Err(BrookletError::InvalidServiceUrl(
            "service URL must use HTTPS".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BrookletError::InvalidServiceUrl(
            "service URL must not include credentials".into(),
        ));
    }
    if url.host_str().is_none() {
        return Err(BrookletError::InvalidServiceUrl(
            "service URL must include a host".into(),
        ));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(BrookletError::InvalidServiceUrl(
            "service URL must not include a query or fragment".into(),
        ));
    }
    Ok(url)
}

pub fn canonical_url(value: &str) -> Result<String, url::ParseError> {
    let mut url = Url::parse(value.trim())?;
    url.set_fragment(None);
    if matches!(
        (url.scheme(), url.port()),
        ("https", Some(443)) | ("http", Some(80))
    ) {
        let _ = url.set_port(None);
    }
    if url.path().len() > 1 {
        let trimmed = url.path().trim_end_matches('/').to_owned();
        url.set_path(if trimmed.is_empty() { "/" } else { &trimmed });
    }
    Ok(url.to_string())
}

pub fn resolve_http_url(base: Option<&Url>, candidate: &str) -> Option<String> {
    let parsed = Url::parse(candidate)
        .or_else(|_| {
            base.ok_or(url::ParseError::RelativeUrlWithoutBase)?
                .join(candidate)
        })
        .ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
}

pub fn validate_article_image_url(value: &str) -> Result<Url, BrookletError> {
    let url = Url::parse(value)
        .map_err(|_| BrookletError::InvalidServiceUrl("image URL is not valid".into()))?;
    if url.scheme() != "https" {
        return Err(BrookletError::InvalidServiceUrl(
            "article image URL must use HTTPS".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(BrookletError::InvalidServiceUrl(
            "article image URL must not include credentials".into(),
        ));
    }
    let literal_address = match url.host() {
        Some(Host::Ipv4(address)) => Some(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => Some(IpAddr::V6(address)),
        _ => None,
    };
    if literal_address.is_some_and(|address| !is_public_ip(address)) {
        return Err(BrookletError::InvalidServiceUrl(
            "article images cannot use non-public network addresses".into(),
        ));
    }
    Ok(url)
}

pub fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || a == 0
        || (a == 100 && (64..=127).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 88 && c == 99)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return is_public_ipv4(mapped);
    }
    let first = address.octets()[0];
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        || (first & 0xfe) == 0xfc
        || (address.segments()[0] & 0xffc0) == 0xfe80
        || (first & 0xe0) != 0x20
        || address.segments()[0] == 0x2001 && address.segments()[1] == 0x0db8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_urls_normalize_origin_and_discard_fragments() {
        assert_eq!(
            canonical_url("HTTPS://Example.COM:443/story/?a=1#comments").unwrap(),
            "https://example.com/story?a=1"
        );
    }

    #[test]
    fn configured_services_require_https_without_credentials() {
        assert!(service_url("http://example.com").is_err());
        assert!(service_url("https://name:secret@example.com").is_err());
        assert!(service_url("https://example.com/miniflux?token=secret").is_err());
        assert!(service_url("https://example.com/miniflux#section").is_err());
        assert!(service_url("https://example.com/miniflux/").is_ok());
    }

    #[test]
    fn relative_http_links_resolve_and_unsafe_schemes_do_not() {
        let base = Url::parse("https://example.com/news/story").unwrap();
        assert_eq!(
            resolve_http_url(Some(&base), "/review/mini-pc").as_deref(),
            Some("https://example.com/review/mini-pc")
        );
        assert_eq!(resolve_http_url(Some(&base), "javascript:alert(1)"), None);
    }

    #[test]
    fn image_policy_rejects_non_public_ranges() {
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "172.16.0.1",
            "192.168.1.1",
            "100.64.0.1",
            "224.0.0.1",
            "::",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd00::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "198.18.0.1",
            "198.51.100.1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn dns_results_are_the_exact_validated_connector_addresses() {
        let public = "1.1.1.1:443".parse().unwrap();
        let private = "10.0.0.1:443".parse().unwrap();
        assert_eq!(validate_dns_answers(vec![public]).unwrap(), vec![public]);
        assert!(validate_dns_answers(vec![public, private]).is_err());
        assert!(validate_dns_answers(Vec::new()).is_err());
    }

    #[test]
    fn image_response_limit_applies_across_chunks() {
        let mut bytes = vec![0; MAX_ARTICLE_IMAGE_BYTES - 1];
        append_image_chunk(&mut bytes, &[1]).unwrap();
        assert_eq!(bytes.len(), MAX_ARTICLE_IMAGE_BYTES);
        assert!(append_image_chunk(&mut bytes, &[2]).is_err());
    }

    #[test]
    fn image_urls_are_https_and_literal_hosts_are_checked() {
        assert!(validate_article_image_url("http://example.com/image.jpg").is_err());
        assert!(validate_article_image_url("https://127.0.0.1/image.jpg").is_err());
        assert!(validate_article_image_url("https://[fd00::1]/image.jpg").is_err());
        assert!(validate_article_image_url("https://example.com/image.jpg").is_ok());
    }
}
