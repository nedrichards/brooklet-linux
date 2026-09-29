use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node, Selector};
use url::Url;

use crate::{
    model::{DocumentBlock, Inline, TableCellLayout},
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
                append_nested_assets(&element, base, blocks);
            }
            "p" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(rich_block(&element, base, RichKind::Paragraph));
                append_nested_assets(&element, base, blocks);
            }
            "blockquote" => {
                flush_loose(&mut loose, base, blocks);
                let mut quoted = Vec::new();
                walk_container(child, base, &mut quoted);
                blocks.extend(quoted.into_iter().map(|block| match block {
                    DocumentBlock::Paragraph { text, inline } => {
                        DocumentBlock::Quote { text, inline }
                    }
                    DocumentBlock::ListItem {
                        text,
                        inline,
                        ordered,
                        ordinal,
                        depth,
                    } => DocumentBlock::ListItem {
                        text,
                        inline,
                        ordered,
                        ordinal,
                        depth: depth + 1,
                    },
                    other => other,
                }));
            }
            "pre" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(DocumentBlock::Code {
                    text: element.text().collect::<String>(),
                });
                append_nested_assets(&element, base, blocks);
            }
            "ol" | "ul" => {
                flush_loose(&mut loose, base, blocks);
                append_list(&element, base, blocks, tag == "ol", 0);
            }
            "figure" => {
                flush_loose(&mut loose, base, blocks);
                walk_container(child, base, blocks);
            }
            "figcaption" => {
                flush_loose(&mut loose, base, blocks);
                blocks.push(rich_block(&element, base, RichKind::Caption));
                append_nested_assets(&element, base, blocks);
            }
            "table" => {
                flush_loose(&mut loose, base, blocks);
                if element
                    .select(&Selector::parse("table").expect("table selector"))
                    .next()
                    .is_some()
                {
                    // Nested tables are typically page layout. Walk them once,
                    // retaining actual leaf tables instead of duplicating their cells.
                    walk_container(child, base, blocks);
                } else {
                    append_table(&element, base, blocks);
                    append_nested_assets(&element, base, blocks);
                }
            }
            "iframe" | "video" | "audio" => {
                flush_loose(&mut loose, base, blocks);
                append_media(&element, base, blocks);
            }
            "img" => {
                flush_loose(&mut loose, base, blocks);
                if let Some(image) = image_block(&element, base) {
                    blocks.push(image);
                }
            }
            "script" | "style" | "noscript" | "template" => {}
            "div" | "article" | "section" | "main" | "body" | "html" | "tbody" | "thead"
            | "tfoot" | "tr" | "td" | "th" => {
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
        RichKind::Caption => DocumentBlock::Caption { text, inline },
    }
}

fn append_list(
    element: &ElementRef<'_>,
    base: Option<&Url>,
    blocks: &mut Vec<DocumentBlock>,
    ordered: bool,
    depth: u32,
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
        let own_children: Vec<_> = item
            .children()
            .filter(|node| {
                ElementRef::wrap(*node)
                    .is_none_or(|element| !matches!(element.value().name(), "ol" | "ul"))
            })
            .collect();
        let text = normalize_whitespace(
            &own_children
                .iter()
                .map(|node| node_text(*node))
                .collect::<Vec<_>>()
                .join(" "),
        );
        let inline = own_children
            .into_iter()
            .flat_map(|node| inline_node(node, base))
            .collect();
        blocks.push(DocumentBlock::ListItem {
            text,
            inline,
            ordered,
            ordinal: ordered.then_some(current),
            depth,
        });
        for child in item.children() {
            let Some(child) = ElementRef::wrap(child) else {
                continue;
            };
            match child.value().name() {
                "ol" | "ul" => append_list(
                    &child,
                    base,
                    blocks,
                    child.value().name() == "ol",
                    depth + 1,
                ),
                "img" => blocks.extend(image_block(&child, base)),
                _ => append_nested_assets(&child, base, blocks),
            }
        }
        if ordered {
            ordinal = current.saturating_add(1);
        }
    }
}

fn append_table(element: &ElementRef<'_>, base: Option<&Url>, blocks: &mut Vec<DocumentBlock>) {
    let row_selector = Selector::parse("tr").expect("static row selector");
    let cell_selector = Selector::parse("th, td").expect("static cell selector");
    let mut rows = Vec::new();
    let mut inline_rows = Vec::new();
    let mut cell_layout = Vec::new();
    let mut occupied = std::collections::HashSet::new();
    let html_rows: Vec<_> = element.select(&row_selector).collect();
    for (row_index, row) in html_rows.iter().enumerate() {
        let cells: Vec<_> = row.select(&cell_selector).collect();
        let text: Vec<_> = cells
            .iter()
            .map(|cell| normalize_whitespace(&cell.text().collect::<Vec<_>>().join(" ")))
            .collect();
        let mut column = 0;
        let mut layout = Vec::new();
        for cell in &cells {
            let column_span = cell
                .value()
                .attr("colspan")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1)
                .clamp(1, 64);
            let row_span = cell
                .value()
                .attr("rowspan")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1);
            let remaining_rows = html_rows[row_index..]
                .iter()
                .take_while(|other| other.parent() == row.parent())
                .count() as u32;
            let row_span = if row_span == 0 {
                // HTML's zero span extends to the end of this row group.
                remaining_rows
            } else {
                row_span
            }
            .clamp(1, remaining_rows.clamp(1, 128));
            while (column..column + column_span)
                .any(|column| occupied.contains(&(row_index as u32, column)))
            {
                column += 1;
            }
            layout.push(TableCellLayout {
                column,
                row_span,
                column_span,
                header: cell.value().name() == "th",
            });
            for row in row_index as u32..row_index as u32 + row_span {
                for col in column..column + column_span {
                    occupied.insert((row, col));
                }
            }
            column += column_span;
        }
        let inline = cells
            .iter()
            .map(|cell| {
                let inline: Vec<_> = cell
                    .children()
                    .flat_map(|node| inline_node(node, base))
                    .collect();
                if cell.value().name() == "th" {
                    vec![Inline::Strong(inline)]
                } else {
                    inline
                }
            })
            .collect();
        rows.push(text);
        inline_rows.push(inline);
        cell_layout.push(layout);
    }
    if rows.iter().flatten().any(|cell| !cell.is_empty()) {
        blocks.push(DocumentBlock::Table {
            rows,
            inline_rows,
            cell_layout,
        });
    }
}

fn append_media(element: &ElementRef<'_>, base: Option<&Url>, blocks: &mut Vec<DocumentBlock>) {
    let source = element.value().attr("src").or_else(|| {
        element
            .select(&Selector::parse("source").expect("source selector"))
            .find_map(|source| source.value().attr("src"))
    });
    let text = match element.value().name() {
        "audio" => "Listen to audio",
        "video" => "Open video",
        _ => "Open embedded content",
    }
    .to_owned();
    if let Some(url) = source.and_then(|source| resolve_http_url(base, source)) {
        blocks.push(DocumentBlock::Paragraph {
            inline: vec![Inline::Link {
                text: vec![Inline::Text(text.clone())],
                url,
            }],
            text,
        });
    }
}

fn append_nested_assets(
    element: &ElementRef<'_>,
    base: Option<&Url>,
    blocks: &mut Vec<DocumentBlock>,
) {
    let selector = Selector::parse("img, iframe, video, audio").expect("static asset selector");
    for asset in element.select(&selector) {
        if asset.value().name() == "img" {
            blocks.extend(image_block(&asset, base));
        } else {
            append_media(&asset, base, blocks);
        }
    }
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
                "sup" => vec![Inline::Superscript(children())],
                "sub" => vec![Inline::Subscript(children())],
                "s" | "del" => vec![Inline::Strikethrough(children())],
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
        DocumentBlock::Table { rows, .. } => rows.is_empty(),
        DocumentBlock::Image { url, .. } => url.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_spans_reserve_columns_and_stop_at_row_group_boundaries() {
        let blocks = parse_document(
            "<table><thead><tr><th colspan='2'>Heading</th></tr></thead><tbody><tr><td rowspan='0'>A</td><td>B</td></tr><tr><td>C</td></tr></tbody><tbody><tr><td>D</td><td>E</td></tr></tbody></table>",
            None,
        );
        let DocumentBlock::Table { cell_layout, .. } = &blocks[0] else {
            panic!("table expected")
        };
        assert_eq!(
            cell_layout[0][0],
            TableCellLayout {
                column: 0,
                row_span: 1,
                column_span: 2,
                header: true
            }
        );
        assert_eq!(cell_layout[1][0].row_span, 2);
        assert_eq!(cell_layout[2][0].column, 1);
        assert_eq!(cell_layout[3][0].column, 0);
        let mut occupied = std::collections::HashSet::new();
        for (row, cells) in cell_layout.iter().enumerate() {
            for cell in cells {
                for y in row as u32..row as u32 + cell.row_span {
                    for x in cell.column..cell.column + cell.column_span {
                        assert!(occupied.insert((y, x)), "merged cells must not overlap");
                    }
                }
            }
        }
    }

    #[test]
    fn table_cells_preserve_headers_links_and_emphasis() {
        let blocks = parse_document(
            "<table><tr><th>Heading</th></tr><tr><td><a href='/detail'><em>Detail</em></a></td></tr></table>",
            Some("https://example.com/story"),
        );
        assert!(
            matches!(&blocks[0], DocumentBlock::Table { inline_rows, .. } if matches!(&inline_rows[0][0][0], Inline::Strong(_)) && matches!(&inline_rows[1][0][0], Inline::Link { url, text } if url == "https://example.com/detail" && matches!(&text[0], Inline::Emphasis(_))))
        );
    }

    #[test]
    fn retains_nested_lists_and_quoted_block_structure() {
        let blocks = parse_document(
            "<blockquote><p>First paragraph.</p><ul><li>Parent<ul><li>Child</li></ul></li><li>Sibling</li></ul><p>Last paragraph.</p></blockquote>",
            None,
        );
        assert!(
            matches!(&blocks[0], DocumentBlock::Quote { text, .. } if text == "First paragraph.")
        );
        let items: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                DocumentBlock::ListItem { text, depth, .. } => Some((text.as_str(), *depth)),
                _ => None,
            })
            .collect();
        assert_eq!(items, [("Parent", 1), ("Child", 2), ("Sibling", 1)]);
        assert!(
            matches!(blocks.last().unwrap(), DocumentBlock::Quote { text, .. } if text == "Last paragraph.")
        );
    }

    #[test]
    fn retains_caption_images_and_empty_table_columns() {
        let blocks = parse_document(
            "<figure><figcaption>Caption<img src='/caption.png'></figcaption></figure><table><tr><td>A</td><td></td><td>C<img src='/cell.png'></td></tr></table>",
            Some("https://example.com/story"),
        );
        let images: Vec<_> = blocks
            .iter()
            .filter_map(|block| match block {
                DocumentBlock::Image { url, .. } => Some(url.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            images,
            [
                "https://example.com/caption.png",
                "https://example.com/cell.png"
            ]
        );
        assert!(blocks.iter().any(|block| matches!(block, DocumentBlock::Table { rows, .. } if rows == &vec![vec!["A".to_string(), "".to_string(), "C".to_string()]])));
    }

    #[test]
    fn media_has_safe_visible_links_and_inline_typography_survives() {
        let blocks = parse_document(
            "<p>x<sup>2</sup> H<sub>2</sub>O <del>old</del></p><iframe src='/embed'></iframe><audio><source src='/audio.mp3'></audio><video src='javascript:bad'></video>",
            Some("https://example.com/story"),
        );
        assert!(
            matches!(&blocks[0], DocumentBlock::Paragraph { inline, .. } if inline.iter().any(|part| matches!(part, Inline::Superscript(_))) && inline.iter().any(|part| matches!(part, Inline::Subscript(_))) && inline.iter().any(|part| matches!(part, Inline::Strikethrough(_))))
        );
        assert_eq!(blocks.len(), 3);
        assert!(
            matches!(&blocks[1], DocumentBlock::Paragraph { inline, .. } if matches!(&inline[0], Inline::Link { url, .. } if url == "https://example.com/embed"))
        );
        assert!(
            matches!(&blocks[2], DocumentBlock::Paragraph { text, .. } if text == "Listen to audio")
        );
    }

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
                .any(|block| matches!(block, DocumentBlock::Table { rows, .. } if rows.len() == 2))
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
