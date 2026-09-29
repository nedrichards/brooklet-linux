use brooklet::{
    model::{DocumentBlock, FailureKind, Inline, classify_http_status, incremental_start},
    reader::parse_document,
    services::url_policy::canonical_url,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct ParserCase {
    name: String,
    html: String,
    base_url: Option<String>,
    text_contains: Vec<String>,
    image_urls: Vec<String>,
    links: Vec<String>,
    ordinals: Vec<i64>,
    caption: Option<String>,
    table_rows: usize,
}

#[derive(Deserialize)]
struct CoreContracts {
    canonical_urls: Vec<CanonicalCase>,
    cursor_overlap: Vec<CursorCase>,
    retryable_statuses: Vec<u16>,
    authentication_statuses: Vec<u16>,
    malformed_statuses: Vec<u16>,
}

#[derive(Deserialize)]
struct CanonicalCase {
    input: String,
    expected: String,
}

#[derive(Deserialize)]
struct CursorCase {
    cursor: i64,
    expected: i64,
}

#[test]
fn android_derived_html_fixtures_match() {
    let cases: Vec<ParserCase> =
        serde_json::from_str(include_str!("fixtures/html-parser.json")).unwrap();
    for case in cases {
        let blocks = parse_document(&case.html, case.base_url.as_deref());
        let texts: Vec<_> = blocks.iter().filter_map(block_text).collect();
        for expected in &case.text_contains {
            assert!(
                texts.contains(&expected.as_str()),
                "{}: missing text {expected:?} in {texts:?}",
                case.name
            );
        }
        let images: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                DocumentBlock::Image { url, .. } => Some(url.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(images, case.image_urls, "{}: image URLs", case.name);

        let mut links = Vec::new();
        for block in &blocks {
            if let Some(inline) = block_inline(block) {
                collect_links(inline, &mut links);
            }
            if let DocumentBlock::Table { inline_rows, .. } = block {
                for cell in inline_rows.iter().flatten() {
                    collect_links(cell, &mut links);
                }
            }
        }
        assert_eq!(links, case.links, "{}: links", case.name);

        let ordinals: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                DocumentBlock::ListItem { ordinal, .. } => *ordinal,
                _ => None,
            })
            .collect();
        assert_eq!(ordinals, case.ordinals, "{}: list ordinals", case.name);

        if let Some(expected) = case.caption {
            assert!(blocks.iter().any(|block| matches!(block, DocumentBlock::Caption { text, .. } if text == &expected)), "{}: caption", case.name);
        }
        if case.table_rows > 0 {
            assert!(blocks.iter().any(|block| matches!(block, DocumentBlock::Table { rows, .. } if rows.len() == case.table_rows)), "{}: table", case.name);
        }
    }
}

#[test]
fn android_derived_core_rule_fixtures_match() {
    let contracts: CoreContracts =
        serde_json::from_str(include_str!("fixtures/core-contracts.json")).unwrap();
    for case in contracts.canonical_urls {
        assert_eq!(canonical_url(&case.input).unwrap(), case.expected);
    }
    for case in contracts.cursor_overlap {
        assert_eq!(incremental_start(Some(case.cursor), 60), case.expected);
    }
    for status in contracts.retryable_statuses {
        assert_eq!(classify_http_status(status), FailureKind::Retryable);
    }
    for status in contracts.authentication_statuses {
        assert_eq!(classify_http_status(status), FailureKind::Authentication);
    }
    for status in contracts.malformed_statuses {
        assert_eq!(classify_http_status(status), FailureKind::MalformedRequest);
    }
}

fn block_text(block: &DocumentBlock) -> Option<&str> {
    match block {
        DocumentBlock::Heading { text, .. }
        | DocumentBlock::Paragraph { text, .. }
        | DocumentBlock::Quote { text, .. }
        | DocumentBlock::Code { text }
        | DocumentBlock::ListItem { text, .. }
        | DocumentBlock::Caption { text, .. } => Some(text),
        DocumentBlock::Table { .. } | DocumentBlock::Image { .. } => None,
    }
}

fn block_inline(block: &DocumentBlock) -> Option<&[Inline]> {
    match block {
        DocumentBlock::Heading { inline, .. }
        | DocumentBlock::Paragraph { inline, .. }
        | DocumentBlock::Quote { inline, .. }
        | DocumentBlock::ListItem { inline, .. }
        | DocumentBlock::Caption { inline, .. } => Some(inline),
        DocumentBlock::Code { .. } | DocumentBlock::Table { .. } | DocumentBlock::Image { .. } => {
            None
        }
    }
}

fn collect_links<'a>(inline: &'a [Inline], links: &mut Vec<&'a str>) {
    for item in inline {
        match item {
            Inline::Link { text, url } => {
                links.push(url);
                collect_links(text, links);
            }
            Inline::Strong(children)
            | Inline::Emphasis(children)
            | Inline::Superscript(children)
            | Inline::Subscript(children)
            | Inline::Strikethrough(children) => collect_links(children, links),
            Inline::Text(_) | Inline::Code(_) | Inline::Break => {}
        }
    }
}
