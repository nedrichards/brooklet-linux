use adw::prelude::*;
use brooklet::{
    model::{DocumentBlock, Entry, Inline},
    reader::parse_document,
};

pub struct ImageSlot {
    pub url: String,
    pub container: gtk::Box,
    pub alt: String,
}

pub fn show(entry: &Entry, title: &adw::WindowTitle, content: &gtk::Box) -> Vec<ImageSlot> {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }

    title.set_title(&entry.title);
    title.set_subtitle(&entry.feed_title);

    let article_title = label(&entry.title, true);
    article_title.add_css_class("title-1");
    content.append(&article_title);

    let mut byline = vec![entry.feed_title.clone()];
    if let Some(author) = entry.author.as_deref() {
        byline.push(author.to_owned());
    }
    if entry.reading_minutes > 0 {
        byline.push(format!("{} min read", entry.reading_minutes));
    }
    let byline = label(&byline.join(" · "), false);
    byline.add_css_class("dim-label");
    content.append(&byline);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let mut images = Vec::new();
    for block in parse_document(&entry.html, Some(&entry.url)) {
        let image = if let DocumentBlock::Image { url, description } = &block {
            Some((
                url.clone(),
                description
                    .clone()
                    .unwrap_or_else(|| "Article image".into()),
            ))
        } else {
            None
        };
        let widget = block_widget(block);
        if let Some((url, alt)) = image {
            images.push(ImageSlot {
                url,
                container: widget
                    .clone()
                    .downcast::<gtk::Box>()
                    .expect("image block is a box"),
                alt,
            });
        }
        content.append(&widget);
    }
    images
}

fn block_widget(block: DocumentBlock) -> gtk::Widget {
    match block {
        DocumentBlock::Heading {
            level,
            text,
            inline,
        } => {
            let widget = rich_label(&text, &inline);
            widget.add_css_class(if level <= 2 { "title-2" } else { "title-3" });
            widget.upcast()
        }
        DocumentBlock::Paragraph { text, inline } => rich_label(&text, &inline).upcast(),
        DocumentBlock::Quote { text, inline } => {
            let widget = rich_label(&text, &inline);
            widget.add_css_class("reader-quote");
            widget.upcast()
        }
        DocumentBlock::Code { text } => {
            let widget = label(&text, true);
            widget.add_css_class("monospace");
            widget.add_css_class("reader-code");
            widget.upcast()
        }
        DocumentBlock::ListItem {
            text,
            ordered,
            ordinal,
            inline,
        } => {
            let marker = if ordered {
                format!("{}. ", ordinal.unwrap_or(1))
            } else {
                "• ".into()
            };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let marker = label(&marker, false);
            marker.set_selectable(false);
            row.append(&marker);
            let body = rich_label(&text, &inline);
            body.set_hexpand(true);
            row.append(&body);
            row.upcast()
        }
        DocumentBlock::Caption { text, inline } => {
            let widget = rich_label(&text, &inline);
            widget.add_css_class("dim-label");
            widget.add_css_class("caption");
            widget.upcast()
        }
        DocumentBlock::Table { rows } => {
            let table = gtk::Box::new(gtk::Orientation::Vertical, 6);
            table.add_css_class("reader-table");
            for row in rows {
                let row = label(&row.join("   ·   "), true);
                table.append(&row);
            }
            table.upcast()
        }
        DocumentBlock::Image { url, description } => {
            let image = gtk::Box::new(gtk::Orientation::Vertical, 8);
            image.add_css_class("reader-image-placeholder");
            let icon = gtk::Image::from_icon_name("image-x-generic-symbolic");
            icon.set_pixel_size(48);
            image.append(&icon);
            let description = description.unwrap_or_else(|| "Article image".into());
            let alt = label(&description, true);
            alt.set_justify(gtk::Justification::Center);
            alt.set_xalign(0.5);
            image.append(&alt);
            let source = label(&url, true);
            source.add_css_class("dim-label");
            source.add_css_class("caption");
            source.set_justify(gtk::Justification::Center);
            source.set_xalign(0.5);
            image.append(&source);
            image.upcast()
        }
    }
}

fn label(text: &str, wrap: bool) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_selectable(true);
    label.set_wrap(wrap);
    if wrap {
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    }
    label
}

fn rich_label(text: &str, inline: &[Inline]) -> gtk::Label {
    let widget = label(text, true);
    if !inline.is_empty() {
        widget.set_markup(&inline_markup(inline));
    }
    widget
}

fn inline_markup(inline: &[Inline]) -> String {
    let mut result = String::new();
    for part in inline {
        match part {
            Inline::Text(value) => result.push_str(&adw::glib::markup_escape_text(value)),
            Inline::Strong(children) => {
                result.push_str(&format!("<b>{}</b>", inline_markup(children)))
            }
            Inline::Emphasis(children) => {
                result.push_str(&format!("<i>{}</i>", inline_markup(children)))
            }
            Inline::Code(value) => result.push_str(&format!(
                "<tt>{}</tt>",
                adw::glib::markup_escape_text(value)
            )),
            Inline::Link { text, url } if matches!(url::Url::parse(url), Ok(parsed) if matches!(parsed.scheme(), "http" | "https")) =>
            {
                result.push_str(&format!(
                    "<a href=\"{}\">{}</a>",
                    adw::glib::markup_escape_text(url),
                    inline_markup(text)
                ));
            }
            Inline::Link { text, .. } => result.push_str(&inline_markup(text)),
            Inline::Break => result.push('\n'),
        }
    }
    result
}
