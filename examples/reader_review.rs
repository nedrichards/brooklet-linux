//! Offline review of an exported Vec<Entry>; never opens the application database or keyring.
use adw::prelude::*;
use brooklet::{model::Entry, reader::parse_document};
use std::{cell::Cell, collections::HashMap, error::Error, path::Path, rc::Rc};
#[path = "../src/ui/reader.rs"]
#[allow(dead_code)] // The app also uses the shared reader's scroll helpers.
mod reader;

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 3 {
        return Err(
            "usage: reader_review CORPUS.json OUTPUT_DIR [--images] [ARTICLE_ID ...]".into(),
        );
    }
    let entries: Vec<Entry> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let output = Path::new(&args[2]);
    std::fs::create_dir_all(output)?;
    let documents: Vec<_> = entries
        .iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id, "blocks": parse_document(&entry.html, Some(&entry.url))
            })
        })
        .collect();
    std::fs::write(output.join("blocks.json"), serde_json::to_vec(&documents)?)?;
    println!("Parsed {} articles.", entries.len());
    if args.len() == 3 {
        return Ok(());
    }
    adw::init()?;
    let offline_images = args.iter().any(|arg| arg == "--offline-images");
    let load_images = offline_images || args.iter().any(|arg| arg == "--images");
    let runtime = tokio::runtime::Runtime::new()?;
    let image_cache = brooklet::services::image_cache::ImageCache::new(
        output.join("images.db"),
        brooklet::services::image_cache::DEFAULT_IMAGE_CACHE_BYTES,
    )?;
    let mut textures = HashMap::<String, Option<gtk::gdk::Texture>>::new();
    let css = gtk::CssProvider::new();
    css.load_from_string(include_str!("../resources/style.css"));
    gtk::style_context_add_provider_for_display(
        &gtk::gdk::Display::default().ok_or("no display")?,
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    for id in args[3..]
        .iter()
        .filter(|arg| *arg != "--images" && *arg != "--offline-images")
    {
        let id: i64 = id.parse()?;
        let entry = entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("unknown article")?;
        for width in [400, 760] {
            let window = adw::Window::builder()
                .default_width(width)
                .default_height(900)
                .title("Brooklet offline reader review")
                .build();
            let toolbar = adw::ToolbarView::new();
            let title = adw::WindowTitle::new("", "");
            let header = adw::HeaderBar::new();
            header.set_title_widget(Some(&title));
            toolbar.add_top_bar(&header);
            let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
            content.set_margin_top(32);
            content.set_margin_bottom(48);
            content.set_margin_start(24);
            content.set_margin_end(24);
            let slots = reader::show(entry, &title, &content);
            for slot in slots {
                if load_images {
                    let texture = textures.entry(slot.url.clone()).or_insert_with(|| {
                        let fetched = runtime.block_on(async {
                            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                                if offline_images {
                                    image_cache
                                        .get_or_fetch(&slot.url, || async {
                                            Err(brooklet::error::BrookletError::InvalidSetup(
                                                "offline preview",
                                            ))
                                        })
                                        .await
                                } else {
                                    image_cache.fetch(&slot.url).await
                                }
                            })
                            .await
                        });
                        let Ok(Ok(bytes)) = fetched else {
                            eprintln!("Article {id}: image unavailable; retained placeholder");
                            return None;
                        };
                        let _entered = runtime.enter();
                        adw::glib::MainContext::default().block_on(async {
                            let mut image = match glycin::Loader::new_vec(bytes).load().await {
                                Ok(image) => image,
                                Err(error) => {
                                    eprintln!("Article {id}: image decode failed: {error}");
                                    return None;
                                }
                            };
                            match image.next_frame().await {
                                Ok(frame) => Some(frame.texture()),
                                Err(error) => {
                                    eprintln!("Article {id}: image frame failed: {error}");
                                    None
                                }
                            }
                        })
                    });
                    if let Some(texture) = texture {
                        while let Some(child) = slot.container.first_child() {
                            slot.container.remove(&child);
                        }
                        let picture = reader::picture(texture, &slot.alt);
                        slot.container.append(&picture);
                    }
                }
            }
            let clamp = adw::Clamp::builder()
                .maximum_size(760)
                .child(&content)
                .build();
            let scroller = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&clamp)
                .build();
            toolbar.set_content(Some(&scroller));
            window.set_content(Some(&toolbar));
            window.present();
            let main_loop = adw::glib::MainLoop::new(None, false);
            let phase = Rc::new(Cell::new(0_usize));
            let missed_frames = Rc::new(Cell::new(0));
            let capture_failed = Rc::new(Cell::new(false));
            let output = output.to_path_buf();
            let mut targets = Vec::<(String, f64, Option<(gtk::Adjustment, f64)>)>::new();
            adw::glib::timeout_add_local(std::time::Duration::from_millis(180), {
                let window = window.clone();
                let main_loop = main_loop.clone();
                let capture_failed = capture_failed.clone();
                move || {
                    let step = phase.get();
                    if targets.is_empty() {
                        let max_scroll = (scroller.vadjustment().upper()
                            - scroller.vadjustment().page_size())
                        .max(0.0);
                        targets.extend([
                            ("0".into(), 0.0, None),
                            ("1".into(), max_scroll / 2.0, None),
                            ("2".into(), max_scroll, None),
                        ]);
                        let mut child = content.first_child();
                        let mut table_index = 0;
                        while let Some(widget) = child {
                            if widget.has_css_class("reader-table-scroller")
                                && let Some(bounds) = widget.compute_bounds(&content)
                            {
                                let horizontal = widget
                                    .first_child()
                                    .and_downcast::<gtk::ScrolledWindow>()
                                    .map(|scroller| scroller.hadjustment());
                                let top = (bounds.y() as f64 + content.margin_top() as f64)
                                    .min(max_scroll);
                                targets.push((
                                    format!("table-{table_index}"),
                                    top,
                                    horizontal.clone().map(|adjustment| (adjustment, 0.0)),
                                ));
                                if let Some(adjustment) = horizontal {
                                    let right = adjustment.upper() - adjustment.page_size();
                                    if right > 1.0 {
                                        targets.push((
                                            format!("table-{table_index}-right"),
                                            top,
                                            Some((adjustment, right)),
                                        ));
                                    }
                                }
                                table_index += 1;
                            }
                            child = widget.next_sibling();
                        }
                    }
                    if step.is_multiple_of(2) {
                        scroller.vadjustment().set_value(targets[step / 2].1);
                        if let Some((adjustment, value)) = &targets[step / 2].2 {
                            adjustment.set_value(*value);
                        }
                    } else {
                        let snapshot = gtk::Snapshot::new();
                        gtk::WidgetPaintable::new(Some(&window)).snapshot(
                            &snapshot,
                            window.width() as f64,
                            window.height() as f64,
                        );
                        let Some(node) = snapshot.to_node() else {
                            let missed = missed_frames.get() + 1;
                            missed_frames.set(missed);
                            window.queue_draw();
                            if missed >= 20 {
                                capture_failed.set(true);
                                main_loop.quit();
                                return adw::glib::ControlFlow::Break;
                            }
                            return adw::glib::ControlFlow::Continue;
                        };
                        missed_frames.set(0);
                        let texture = window
                            .renderer()
                            .expect("renderer")
                            .render_texture(&node, None);
                        texture
                            .save_to_png(
                                output.join(format!("{id}-{width}-{}.png", targets[step / 2].0)),
                            )
                            .expect("save screenshot");
                    }
                    phase.set(step + 1);
                    if step + 1 == targets.len() * 2 {
                        main_loop.quit();
                        adw::glib::ControlFlow::Break
                    } else {
                        adw::glib::ControlFlow::Continue
                    }
                }
            });
            main_loop.run();
            window.close();
            if capture_failed.get() {
                return Err("native preview window did not render".into());
            }
        }
    }
    if load_images {
        println!(
            "Images: {} decoded, {} unavailable.",
            textures
                .values()
                .filter(|texture| texture.is_some())
                .count(),
            textures
                .values()
                .filter(|texture| texture.is_none())
                .count()
        );
    }
    Ok(())
}
