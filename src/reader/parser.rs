use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

use crate::{
    model::{DocumentBlock, Inline},
    services::url_policy::resolve_http_url,
};

pub fn parse_document(html: &str, article_url: Option<&str>) -> Vec<DocumentBlock> {
    let document = Html::parse_fragment(html);
    let base = article_url.and_then(|value| Url::parse(value).ok());
    let mut blocks = Vec::new();
    walk_container(*document.root_element(), base.as_ref(), &mut blocks);
    blocks.retain(|block| !block_is_blank(block));
    if blocks.is_empty() {
        let text =
            normalize_whitespace(&document.root_element().text().collect::<Vec<_>>().join(" "));
        if !text.is_empty() {
            blocks.push(DocumentBlock::Paragraph {
                inline: vec![Inline::Text(text.clone())],
                text,
            });
        }
    }
    blocks
}

fn walk_container(parent: NodeRef<'_, Node>, base: Option<&Url>, blocks: &mut Vec<DocumentBlock>) {
    let mut loose = Vec::new();
    for child in parent.children() {
        let Some(element) = ElementRef::wrap(child) else {
            if matches!(child.value(), Node::Text(_)) {
                loose.push(child);
            }
            continue;
        };
        let tag = element.value().name();
        match tag {
            "br" => flush_loose(&mut loose, base, blocks),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush_loose(&mut loose, base, blocks);
                let level = tag[1..].parse().unwrap_or(1);
                blocks.push(rich_block(&element, base, RichKind::Heading(level)));
                append_nested_images(&element, base, blocks);
            }
            "p" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(rich_block(&element, base, RichKind::Paragraph));
                append_nested_images(&element, base, blocks);
            }
            "blockquote" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(rich_block(&element, base, RichKind::Quote));
                append_nested_images(&element, base, blocks);
            }
            "pre" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(DocumentBlock::Code {
                    text: element.text().collect::<String>(),
                });
            }
            "ol" | "ul" => {
                flush_loose(&mut loose, base, blocks);
                append_list(&element, base, blocks, tag == "ol");
            }
            "figure" => {
                flush_loose(&mut loose, base, blocks);
                walk_container(child, base, blocks);
            }
            "figcaption" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(rich_block(&element, base, RichKind::Caption));
            }
            "table" => {
                flush_loose(&mut loose, base, blocks);
                append_table(&element, blocks);
            }
            "img" => {
                flush_loose(&mut loose, base, blocks);
                if let Some(image) = image_block(&element, base) {
                    blocks.push(image);
                }
            }
            "script" | "style" | "noscript" | "template" => {}
            "div" | "article" | "section" | "main" | "body" | "html" => {
                flush_loose(&mut loose, base, blocks);
                walk_container(child, base, blocks);
            }
            _ => loose.push(child),
        }
    }
    flush_loose(&mut loose, base, blocks);
}

fn flush_loose(
    nodes: &mut Vec<NodeRef<'_, Node>>,
    base: Option<&Url>,
    blocks: &mut Vec<DocumentBlock>,
) {
    if nodes.is_empty() {
        return;
    }
    let text = normalize_whitespace(
        &nodes
            .iter()
            .map(|node| node_text(*node))
            .collect::<Vec<_>>()
            .join(" "),
    );
    if !text.is_empty() {
        let inline = nodes
            .iter()
            .flat_map(|node| inline_node(*node, base))
            .collect();
        blocks.push(DocumentBlock::Paragraph { text, inline });
    }
    let selector = Selector::parse("img").expect("static image selector");
    for node in nodes.iter() {
        let Some(element) = ElementRef::wrap(*node) else {
            continue;
        };
        if element.value().name() == "img" {
            if let Some(image) = image_block(&element, base) {
                blocks.push(image);
            }
        } else {
            blocks.extend(
                element
                    .select(&selector)
                    .filter_map(|image| image_block(&image, base)),
            );
        }
    }
    nodes.clear();
}

enum RichKind {
    Heading(u8),
    Paragraph,
    Quote,
    Caption,
}

fn rich_block(element: &ElementRef<'_>, base: Option<&Url>, kind: RichKind) -> DocumentBlock {
    let text = normalize_whitespace(&element.text().collect::<Vec<_>>().join(" "));
    let inline = element
        .children()
        .flat_map(|node| inline_node(node, base))
        .collect();
    match kind {
        RichKind::Heading(level) => DocumentBlock::Heading {
            level,
            text,
            inline,
        },
        RichKind::Paragraph => DocumentBlock::Paragraph { text, inline },
        RichKind::Quote => DocumentBlock::Quote { text, inline },
        RichKind::Caption => DocumentBlock::Caption { text, inline },
    }
}

fn append_list(
    element: &ElementRef<'_>,
    base: Option<&Url>,
    blocks: &mut Vec<DocumentBlock>,
    ordered: bool,
) {
    let mut ordinal = element
        .value()
        .attr("start")
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(1);
    for child in element.children() {
        let Some(item) = ElementRef::wrap(child) else {
            continue;
        };
        if item.value().name() != "li" {
            continue;
        }
        let current = if ordered {
            item.value()
                .attr("value")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(ordinal)
        } else {
            0
        };
        let text = normalize_whitespace(&item.text().collect::<Vec<_>>().join(" "));
        let inline = item
            .children()
            .filter(|node| {
                ElementRef::wrap(*node)
                    .is_none_or(|element| !matches!(element.value().name(), "ol" | "ul"))
            })
            .flat_map(|node| inline_node(node, base))
            .collect();
        blocks.push(DocumentBlock::ListItem {
            text,
            inline,
            ordered,
            ordinal: ordered.then_some(current),
        });
        if ordered {
            ordinal = current.saturating_add(1);
        }
    }
}

fn append_table(element: &ElementRef<'_>, blocks: &mut Vec<DocumentBlock>) {
    let row_selector = Selector::parse("tr").expect("static row selector");
    let cell_selector = Selector::parse("th, td").expect("static cell selector");
    let rows = element
        .select(&row_selector)
        .filter_map(|row| {
            let cells: Vec<_> = row
                .select(&cell_selector)
                .map(|cell| normalize_whitespace(&cell.text().collect::<Vec<_>>().join(" ")))
                .filter(|cell| !cell.is_empty())
                .collect();
            (!cells.is_empty()).then_some(cells)
        })
        .collect();
    blocks.push(DocumentBlock::Table { rows });
}

fn append_nested_images(
    element: &ElementRef<'_>,
    base: Option<&Url>,
    blocks: &mut Vec<DocumentBlock>,
) {
    let selector = Selector::parse("img").expect("static image selector");
    blocks.extend(
        element
            .select(&selector)
            .filter_map(|image| image_block(&image, base)),
    );
}

fn image_block(element: &ElementRef<'_>, base: Option<&Url>) -> Option<DocumentBlock> {
    let source = element
        .value()
        .attr("src")
        .or_else(|| element.value().attr("data-src"))?
        .trim();
    let url = resolve_http_url(base, source)
        .or_else(|| (base.is_none() && !source.is_empty()).then(|| source.to_owned()))?;
    let description = element
        .value()
        .attr("alt")
        .map(normalize_whitespace)
        .filter(|value| !value.is_empty());
    Some(DocumentBlock::Image { url, description })
}

fn inline_node(node: NodeRef<'_, Node>, base: Option<&Url>) -> Vec<Inline> {
    match node.value() {
        Node::Text(text) => vec![Inline::Text(text.text.to_string())],
        Node::Element(_) => {
            let Some(element) = ElementRef::wrap(node) else {
                return Vec::new();
            };
            let children = || {
                element
                    .children()
                    .flat_map(|child| inline_node(child, base))
                    .collect::<Vec<_>>()
            };
            match element.value().name() {
                "strong" | "b" => vec![Inline::Strong(children())],
                "em" | "i" => vec![Inline::Emphasis(children())],
                "code" => vec![Inline::Code(element.text().collect::<String>())],
                "a" => {
                    let content = children();
                    match element
                        .value()
                        .attr("href")
                        .and_then(|href| resolve_http_url(base, href))
                    {
                        Some(url) => vec![Inline::Link { text: content, url }],
                        None => content,
                    }
                }
                "br" => vec![Inline::Break],
                "img" | "script" | "style" | "noscript" | "template" => Vec::new(),
                _ => children(),
            }
        }
        _ => Vec::new(),
    }
}

fn node_text(node: NodeRef<'_, Node>) -> String {
    match node.value() {
        Node::Text(text) => text.text.to_string(),
        Node::Element(_) => ElementRef::wrap(node)
            .map(|element| element.text().collect::<Vec<_>>().join(" "))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn block_is_blank(block: &DocumentBlock) -> bool {
    match block {
        DocumentBlock::Heading { text, .. }
        | DocumentBlock::Paragraph { text, .. }
        | DocumentBlock::Quote { text, .. }
        | DocumentBlock::Code { text }
        | DocumentBlock::ListItem { text, .. }
        | DocumentBlock::Caption { text, .. } => text.is_empty(),
        DocumentBlock::Table { rows } => rows.is_empty(),
        DocumentBlock::Image { url, .. } => url.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_entities_once() {
        let blocks = parse_document(
            "<p>Phoronix &#34;Sched&#x5f;ext&#34; &amp;#34; &#x1F680;</p>",
            None,
        );
        assert_eq!(
            paragraph_text(&blocks[0]),
            "Phoronix \"Sched_ext\" &#34; 🚀"
        );
    }

    #[test]
    fn retains_nested_and_lazy_images() {
        let blocks = parse_document(
            "<p>Before <img src='/photo.jpg' alt='A view'> after</p><img data-src=https://example.com/lazy.jpg alt=Lazy>",
            Some("https://example.com/story"),
        );
        assert_eq!(paragraph_text(&blocks[0]), "Before after");
        assert!(
            matches!(&blocks[1], DocumentBlock::Image { url, description } if url == "https://example.com/photo.jpg" && description.as_deref() == Some("A view"))
        );
        assert!(
            matches!(&blocks[2], DocumentBlock::Image { url, .. } if url == "https://example.com/lazy.jpg")
        );
    }

    #[test]
    fn keeps_loose_feed_prose_and_safe_links() {
        let blocks = parse_document(
            "<a href=https://example.com/article><img src=https://example.com/article.jpg></a><br>Wine 11.16 is out with an <a href=/driver>LED driver</a>.",
            Some("https://example.com/news/story"),
        );
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, DocumentBlock::Image { .. }))
        );
        let paragraph = blocks.iter().find(|block| matches!(block, DocumentBlock::Paragraph { text, .. } if text.contains("Wine 11.16"))).unwrap();
        assert!(
            matches!(paragraph, DocumentBlock::Paragraph { inline, .. } if inline.iter().any(|item| matches!(item, Inline::Link { url, .. } if url == "https://example.com/driver")))
        );
    }

    #[test]
    fn preserves_ordered_list_values_captions_and_tables() {
        let blocks = parse_document(
            "<ol start='3'><li>Three</li><li value=7>Seven</li><li>Eight</li></ol><figure><img src='https://example.com/x'><figcaption>A <em>caption</em></figcaption></figure><table><tr><th>Version</th><th>Status</th></tr><tr><td>1</td><td>Ready</td></tr></table>",
            None,
        );
        let ordinals: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                DocumentBlock::ListItem { ordinal, .. } => *ordinal,
                _ => None,
            })
            .collect();
        assert_eq!(ordinals, [3, 7, 8]);
        assert!(blocks.iter().any(
            |block| matches!(block, DocumentBlock::Caption { text, .. } if text == "A caption")
        ));
        assert!(
            blocks
                .iter()
                .any(|block| matches!(block, DocumentBlock::Table { rows } if rows.len() == 2))
        );
    }

    #[test]
    fn strips_unsafe_link_targets_but_keeps_text() {
        let blocks = parse_document(
            "<p><strong>Safe</strong> and <a href='javascript:alert(1)'><em>visible</em></a> text.</p>",
            None,
        );
        assert_eq!(paragraph_text(&blocks[0]), "Safe and visible text.");
        assert!(
            matches!(&blocks[0], DocumentBlock::Paragraph { inline, .. } if !inline.iter().any(|item| matches!(item, Inline::Link { .. })))
        );
    }

    fn paragraph_text(block: &DocumentBlock) -> &str {
        match block {
            DocumentBlock::Paragraph { text, .. } => text,
            _ => panic!("expected paragraph"),
        }
    }
}
