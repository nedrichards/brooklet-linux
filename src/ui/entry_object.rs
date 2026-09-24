use std::cell::OnceCell;

use adw::glib;
use adw::glib::subclass::prelude::*;
use brooklet::model::Entry;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct EntryObject {
        pub entry: OnceCell<Entry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for EntryObject {
        const NAME: &'static str = "BrookletEntryObject";
        type Type = super::EntryObject;
    }

    impl ObjectImpl for EntryObject {}
}

glib::wrapper! {
    pub struct EntryObject(ObjectSubclass<imp::EntryObject>);
}

impl EntryObject {
    pub fn new(entry: Entry) -> Self {
        let object: Self = glib::Object::builder().build();
        object
            .imp()
            .entry
            .set(entry)
            .expect("entry is initialized exactly once");
        object
    }

    pub fn entry(&self) -> &Entry {
        self.imp().entry.get().expect("entry must be initialized")
    }
}
