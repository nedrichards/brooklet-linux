use std::{cell::RefCell, rc::Rc};

use adw::{glib, prelude::*};
use brooklet::model::ReaderPosition;

pub struct Outline {
    button: gtk::MenuButton,
}

impl Outline {
    pub fn install(
        header: &adw::HeaderBar,
        content: &gtk::Box,
        scroller: &gtk::ScrolledWindow,
    ) -> Self {
        let button = gtk::MenuButton::builder()
            .icon_name("view-list-symbolic")
            .tooltip_text("In this article")
            .visible(false)
            .build();
        button.update_property(&[gtk::accessible::Property::Label("In this article")]);
        header.pack_start(&button);

        let restore = Rc::new(RefCell::new(None::<ReaderPosition>));
        let popover = gtk::Popover::new();
        popover.connect_map({
            let content = content.downgrade();
            let scroller = scroller.downgrade();
            let restore = restore.clone();
            move |popover| {
                let (Some(content), Some(scroller)) = (content.upgrade(), scroller.upgrade())
                else {
                    return;
                };
                populate(popover, &content, &scroller, &restore);
            }
        });
        button.set_popover(Some(&popover));
        // Hiding the control is the document boundary, including failed loads.
        button.connect_visible_notify(move |button| {
            if !button.is_visible() {
                restore.take();
                button.popdown();
            }
        });
        Self { button }
    }

    pub fn reset(&self) {
        self.button.set_visible(false);
    }

    pub fn ready(&self, content: &gtk::Box) {
        let mut count = 0;
        let mut headings = 0;
        let mut child = content.first_child();
        while let Some(widget) = child {
            count += 1;
            headings += usize::from(widget.has_css_class("reader-heading"));
            if count >= 11 && headings >= 2 {
                self.button.set_visible(true);
                return;
            }
            child = widget.next_sibling();
        }
    }
    pub fn check_jump_and_return(
        &self,
        scroller: &gtk::ScrolledWindow,
    ) -> Result<(), glib::BoolError> {
        use super::test_support::wait_for;
        wait_for("article outline button", || self.button.is_mapped())?;
        let adjustment = scroller.vadjustment();
        let original = adjustment.value();
        self.button.popup();
        let popover = self.button.popover().unwrap();
        wait_for("article outline popover", || {
            popover.is_mapped() && popover.child().is_some()
        })?;
        let panel = popover.child().and_downcast::<gtk::Box>().unwrap();
        let scroll = panel
            .first_child()
            .unwrap()
            .next_sibling()
            .and_downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let viewport = scroll.child().and_downcast::<gtk::Viewport>().unwrap();
        let list = viewport.child().and_downcast::<gtk::ListBox>().unwrap();
        let first = list.row_at_index(0).unwrap();
        assert_eq!(list.selected_row().as_ref(), Some(&first));
        list.emit_by_name::<()>("row-activated", &[&first]);
        wait_for("jump to section", || adjustment.value() + 300.0 < original)?;
        self.button.popup();
        wait_for("article outline reopened", || popover.is_mapped())?;
        let panel = popover.child().and_downcast::<gtk::Box>().unwrap();
        let back = panel.last_child().and_downcast::<gtk::Button>().unwrap();
        assert!(back.is_visible());
        back.emit_clicked();
        wait_for("return to reading position", || {
            (adjustment.value() - original).abs() < 2.0
        })
    }
}

fn populate(
    popover: &gtk::Popover,
    content: &gtk::Box,
    scroller: &gtk::ScrolledWindow,
    restore: &Rc<RefCell<Option<ReaderPosition>>>,
) {
    let panel = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let title = gtk::Label::new(Some("In this article"));
    title.add_css_class("heading");
    title.set_margin_top(6);
    panel.append(&title);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("navigation-sidebar");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(340)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    panel.append(&scroll);

    let mut active = None;
    let mut targets = Vec::new();
    let mut child = content.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if !widget.has_css_class("reader-heading") {
            continue;
        }
        let heading = widget
            .downcast::<gtk::Label>()
            .expect("reader heading label");
        let label = gtk::Label::builder()
            .label(heading.text())
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(38)
            .margin_start(if heading.has_css_class("title-2") {
                8
            } else {
                20
            })
            .margin_end(8)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        let row = gtk::ListBoxRow::builder().child(&label).build();
        list.append(&row);
        if heading
            .compute_bounds(scroller)
            .is_some_and(|bounds| bounds.y() <= 24.0)
        {
            active = Some(row);
        }
        targets.push(heading.downgrade());
    }
    list.select_row(active.as_ref());
    list.connect_row_activated({
        let content = content.downgrade();
        let scroller = scroller.downgrade();
        let popover = popover.downgrade();
        let restore = restore.clone();
        move |_, row| {
            let (Some(content), Some(scroller), Some(popover), Some(heading)) = (
                content.upgrade(),
                scroller.upgrade(),
                popover.upgrade(),
                targets
                    .get(row.index() as usize)
                    .and_then(glib::WeakRef::upgrade),
            ) else {
                return;
            };
            if let Some(bounds) = heading.compute_bounds(&scroller) {
                let adjustment = scroller.vadjustment();
                restore.borrow_mut().get_or_insert_with(|| {
                    super::reader::position_from_offset(&content, 0, adjustment.value() as i32)
                });
                adjustment.set_value(adjustment.value() + f64::from(bounds.y()) - 16.0);
                popover.popdown();
            }
        }
    });

    let back = gtk::Button::with_label("Back to reading position");
    back.set_visible(restore.borrow().is_some());
    back.add_css_class("flat");
    back.connect_clicked({
        let content = content.downgrade();
        let scroller = scroller.downgrade();
        let popover = popover.downgrade();
        let restore = restore.clone();
        move |_| {
            if let (Some(content), Some(scroller), Some(popover), Some(position)) = (
                content.upgrade(),
                scroller.upgrade(),
                popover.upgrade(),
                restore.take(),
            ) {
                scroller
                    .vadjustment()
                    .set_value(f64::from(super::reader::offset_from_position(
                        &content, &position,
                    )));
                popover.popdown();
            }
        }
    });
    panel.append(&back);
    popover.set_child(Some(&panel));
}
