use std::cell::{Ref, RefCell};
use std::sync::Arc;

use adw::glib::subclass::prelude::*;
use adw::{glib, prelude::*};
use brooklet::model::Entry;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EntryObject {
        pub entry: RefCell<Option<Arc<Entry>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EntryObject {
        const NAME: &'static str = "BrookletEntryObject";
        type Type = super::EntryObject;
    }

    impl ObjectImpl for EntryObject {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
    }
}

glib::wrapper! {
    pub struct EntryObject(ObjectSubclass<imp::EntryObject>);
}

impl EntryObject {
    pub fn new(entry: Entry) -> Self {
        let object: Self = glib::Object::builder().build();
        *object.imp().entry.borrow_mut() = Some(Arc::new(entry));
        object
    }

    pub fn entry(&self) -> Ref<'_, Entry> {
        Ref::map(self.imp().entry.borrow(), |entry| {
            entry.as_deref().expect("entry must be initialized")
        })
    }

    pub fn update(&self, entry: Entry) {
        assert_eq!(self.entry().id, entry.id, "row identity must remain stable");
        if *self.entry() == entry {
            return;
        }
        *self.imp().entry.borrow_mut() = Some(Arc::new(entry));
        self.emit_by_name::<()>("changed", &[]);
    }

    pub fn shared_entry(&self) -> Arc<Entry> {
        self.imp()
            .entry
            .borrow()
            .as_ref()
            .expect("entry must be initialized")
            .clone()
    }
}
