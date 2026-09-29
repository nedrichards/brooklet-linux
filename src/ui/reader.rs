use adw::prelude::*;
use brooklet::{
    model::{DocumentBlock, Entry, Inline, ReaderPosition},
    reader::parse_document,
};
#[path = "reader_table.rs"]
mod table_view;

mod image_imp {
    use adw::{glib, prelude::*};
    use gtk::subclass::prelude::*;
    use std::cell::OnceCell;

    #[derive(Default)]
    pub struct ArticlePicture {
        pub picture: OnceCell<gtk::Picture>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArticlePicture {
        const NAME: &'static str = "BrookletArticlePicture";
        type Type = super::ArticlePicture;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ArticlePicture {
        fn dispose(&self) {
            if let Some(picture) = self.picture.get()
                && picture.parent().is_some()
            {
                picture.unparent();
            }
        }
    }
    impl WidgetImpl for ArticlePicture {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(paintable) = self.picture.get().and_then(|picture| picture.paintable()) else {
                return (0, 0, -1, -1);
            };
            let width = paintable.intrinsic_width().max(1);
            let height = paintable.intrinsic_height().max(1);
            if orientation == gtk::Orientation::Horizontal {
                (0, width, -1, -1)
            } else if for_size >= 0 {
                // Keep the full proportional height in a vertical scroller,
                // while allowing the image's width to shrink with its pane.
                let height =
                    ((for_size.min(width) as f64 * height as f64) / width as f64).ceil() as i32;
                (height, height, -1, -1)
            } else {
                (0, height, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(picture) = self.picture.get() {
                picture.allocate(width, height, baseline, None);
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(picture) = self.picture.get() {
                self.obj().snapshot_child(picture, snapshot);
            }
        }
    }
}

adw::glib::wrapper! {
    pub struct ArticlePicture(ObjectSubclass<image_imp::ArticlePicture>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

pub fn picture(texture: &gtk::gdk::Texture, alt: &str) -> ArticlePicture {
    use gtk::subclass::prelude::*;
    let container: ArticlePicture = adw::glib::Object::builder().build();
    let picture = gtk::Picture::for_paintable(texture);
    picture.set_alternative_text(Some(alt));
    picture.set_content_fit(gtk::ContentFit::ScaleDown);
    picture.set_can_shrink(true);
    picture.set_parent(&container);
    container
        .imp()
        .picture
        .set(picture)
        .expect("image is initialized once");
    container
}

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
    let byline = label(&byline.join(" · "), true);
    byline.add_css_class("dim-label");
    content.append(&byline);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let mut images = Vec::new();
    let blocks = parse_document(&entry.html, Some(&entry.url));
    if blocks.is_empty() {
        content.append(&label("This article has no cached body.", true));
        if matches!(url::Url::parse(&entry.url), Ok(url) if matches!(url.scheme(), "http" | "https"))
        {
            let link = gtk::LinkButton::with_label(&entry.url, "Open original article");
            link.set_halign(gtk::Align::Start);
            content.append(&link);
        }
    }
    for block in blocks {
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
            depth,
        } => {
            let marker = if ordered {
                format!("{}. ", ordinal.unwrap_or(1))
            } else {
                "• ".into()
            };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.set_margin_start((depth.min(8) * 16) as i32);
            let marker = label(&marker, false);
            marker.set_selectable(false);
            marker.set_valign(gtk::Align::Start);
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
        DocumentBlock::Table {
            rows,
            inline_rows,
            cell_layout,
        } => {
            let table = gtk::Grid::builder()
                .accessible_role(gtk::AccessibleRole::Table)
                .row_spacing(8)
                .column_spacing(16)
                .column_homogeneous(true)
                .build();
            table.add_css_class("reader-table");
            let columns = cell_layout
                .iter()
                .flatten()
                .map(|cell| cell.column + cell.column_span)
                .max()
                .unwrap_or_else(|| rows.iter().map(Vec::len).max().unwrap_or(0) as u32);
            table.update_relation(&[
                gtk::accessible::Relation::RowCount(rows.len() as i32),
                gtk::accessible::Relation::ColCount(columns as i32),
            ]);
            for (row_index, row) in rows.into_iter().enumerate() {
                for (column_index, cell) in row.into_iter().enumerate() {
                    let inline = inline_rows
                        .get(row_index)
                        .and_then(|row| row.get(column_index))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    let cell = rich_label(&cell, inline);
                    cell.set_hexpand(true);
                    cell.set_width_chars(if columns <= 2 { 12 } else { 16 });
                    cell.set_valign(gtk::Align::Start);
                    cell.add_css_class("reader-table-cell");
                    let geometry = cell_layout
                        .get(row_index)
                        .and_then(|row| row.get(column_index));
                    if geometry.is_some_and(|cell| cell.header) {
                        cell.add_css_class("reader-table-header");
                        cell.set_accessible_role(gtk::AccessibleRole::ColumnHeader);
                    } else {
                        cell.set_accessible_role(gtk::AccessibleRole::Cell);
                    }
                    cell.update_relation(&[
                        gtk::accessible::Relation::RowIndex(row_index as i32 + 1),
                        gtk::accessible::Relation::ColIndex(
                            geometry.map_or(column_index as i32, |cell| cell.column as i32) + 1,
                        ),
                        gtk::accessible::Relation::RowSpan(
                            geometry.map_or(1, |cell| cell.row_span as i32),
                        ),
                        gtk::accessible::Relation::ColSpan(
                            geometry.map_or(1, |cell| cell.column_span as i32),
                        ),
                    ]);
                    table.attach(
                        &cell,
                        geometry.map_or(column_index as i32, |cell| cell.column as i32),
                        row_index as i32,
                        geometry.map_or(1, |cell| cell.column_span as i32),
                        geometry.map_or(1, |cell| cell.row_span as i32),
                    );
                }
            }
            table_view::viewport(&table).upcast()
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
            if matches!(url::Url::parse(&url), Ok(url) if matches!(url.scheme(), "http" | "https"))
            {
                let source = gtk::LinkButton::with_label(&url, "Open image");
                source.set_halign(gtk::Align::Center);
                image.append(&source);
            }
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
            Inline::Superscript(children) => {
                result.push_str(&format!("<sup>{}</sup>", inline_markup(children)))
            }
            Inline::Subscript(children) => {
                result.push_str(&format!("<sub>{}</sub>", inline_markup(children)))
            }
            Inline::Strikethrough(children) => {
                result.push_str(&format!("<s>{}</s>", inline_markup(children)))
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

pub fn position_from_offset(content: &gtk::Box, entry_id: i64, scroll: i32) -> ReaderPosition {
    let mut top = 0;
    let mut index = 0;
    let mut child = content.first_child();
    while let Some(widget) = child {
        let height = widget.height() + 18;
        if top + height > scroll {
            break;
        }
        top += height;
        index += 1;
        child = widget.next_sibling();
    }
    ReaderPosition {
        entry_id,
        first_visible_block: index,
        offset_px: scroll.saturating_sub(top),
    }
}

pub fn offset_from_position(content: &gtk::Box, position: &ReaderPosition) -> i32 {
    let mut top = 0;
    let mut child = content.first_child();
    for _ in 0..position.first_visible_block {
        let Some(widget) = child else {
            break;
        };
        top += widget.height() + 18;
        child = widget.next_sibling();
    }
    top + position.offset_px
}

pub fn preserve_after_image(
    content: &gtk::Box,
    scroller: &gtk::ScrolledWindow,
    current_generation: std::rc::Rc<std::cell::Cell<u64>>,
    expected_generation: u64,
    scroll: f64,
    place: ReaderPosition,
) {
    let scroller = scroller.downgrade();
    let frames = std::cell::Cell::new(0);
    content.add_tick_callback(move |content, _| {
        let Some(scroller) = scroller.upgrade() else {
            return adw::glib::ControlFlow::Break;
        };
        if current_generation.get() != expected_generation
            || (scroller.vadjustment().value() - scroll).abs() > 1.0
        {
            return adw::glib::ControlFlow::Break;
        }
        if frames.replace(1) == 0 {
            return adw::glib::ControlFlow::Continue;
        }
        scroller
            .vadjustment()
            .set_value(f64::from(offset_from_position(content, &place)));
        adw::glib::ControlFlow::Break
    });
}
