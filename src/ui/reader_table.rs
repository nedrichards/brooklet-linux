//! A horizontal table viewport whose height follows the allocated reader width.
use adw::{glib, prelude::*};
use gtk::subclass::prelude::*;

mod imp {
    use super::*;
    use std::cell::OnceCell;
    #[derive(Default)]
    pub struct TableViewport {
        pub grid: OnceCell<gtk::Grid>,
        pub scroller: OnceCell<gtk::ScrolledWindow>,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for TableViewport {
        const NAME: &'static str = "BrookletTableViewport";
        type Type = super::TableViewport;
        type ParentType = gtk::Widget;
    }
    impl ObjectImpl for TableViewport {
        fn dispose(&self) {
            if let Some(scroller) = self.scroller.get()
                && scroller.parent().is_some()
            {
                scroller.unparent();
            }
        }
    }
    impl WidgetImpl for TableViewport {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(grid) = self.grid.get() else {
                return (0, 0, -1, -1);
            };
            let (minimum_width, natural_width, _, _) =
                grid.measure(gtk::Orientation::Horizontal, -1);
            if orientation == gtk::Orientation::Horizontal {
                return (0, natural_width, -1, -1);
            }
            let width = if for_size < 0 {
                natural_width
            } else {
                for_size.max(minimum_width)
            };
            let (_, height, _, _) = grid.measure(gtk::Orientation::Vertical, width);
            (height, height, -1, -1)
        }
        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if let Some(scroller) = self.scroller.get() {
                scroller.allocate(width, height, baseline, None);
            }
        }
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            if let Some(scroller) = self.scroller.get() {
                self.obj().snapshot_child(scroller, snapshot);
            }
        }
    }
}

glib::wrapper! {
    pub struct TableViewport(ObjectSubclass<imp::TableViewport>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

pub fn viewport(grid: &gtk::Grid) -> TableViewport {
    let viewport: TableViewport = glib::Object::builder().build();
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .child(grid)
        .build();
    scroller.set_parent(&viewport);
    viewport
        .imp()
        .grid
        .set(grid.clone())
        .expect("table initialized once");
    viewport
        .imp()
        .scroller
        .set(scroller)
        .expect("viewport initialized once");
    viewport.add_css_class("reader-table-scroller");
    viewport
}
