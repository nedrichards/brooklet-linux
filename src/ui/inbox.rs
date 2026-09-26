use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

use adw::{gio, glib, prelude::*};
use brooklet::model::{DeliveryState, Entry};

use super::entry_object::EntryObject;

#[derive(Clone)]
pub struct InboxModel {
    pub store: gio::ListStore,
    pub selection: gtk::SingleSelection,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct InboxChanges {
    pub added: usize,
    pub updated: usize,
}

impl InboxChanges {
    pub fn between(before: &[Entry], after: &[Entry]) -> Self {
        let previous = before
            .iter()
            .map(|entry| (entry.id, entry))
            .collect::<HashMap<_, _>>();
        Self {
            added: after
                .iter()
                .filter(|entry| !previous.contains_key(&entry.id))
                .count(),
            updated: after
                .iter()
                .filter(|entry| previous.get(&entry.id).is_some_and(|old| *old != *entry))
                .count(),
        }
    }

    pub fn toast_message(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.added > 0 {
            parts.push(format!(
                "{} new article{}",
                self.added,
                if self.added == 1 { "" } else { "s" }
            ));
        }
        if self.updated > 0 {
            parts.push(format!("{} updated", self.updated));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }
}

pub fn configure(list: &gtk::ListView) -> InboxModel {
    configure_with_action(list, true)
}

pub fn configure_with_action(list: &gtk::ListView, mark_read_action: bool) -> InboxModel {
    let store = gio::ListStore::new::<EntryObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    list.set_model(Some(&selection));
    // GTK's single-click mode also selects rows on hover. Keep the keyboard
    // cursor stable while the pointer moves over the list.
    list.set_single_click_activate(false);

    let factory = gtk::SignalListItemFactory::new();
    let list_weak = list.downgrade();
    factory.connect_setup(move |_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("factory item must be a GtkListItem");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("article-row");
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(12);
        row.set_margin_end(6);
        let focus = gtk::EventControllerFocus::new();
        let focus_item = item.downgrade();
        let focus_list = list_weak.clone();
        focus.connect_enter(move |_| {
            let (Some(item), Some(list)) = (focus_item.upgrade(), focus_list.upgrade()) else {
                return;
            };
            if item.position() != gtk::INVALID_LIST_POSITION
                && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
            {
                selection.set_selected(item.position());
            }
        });
        row.add_controller(focus);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 4);
        content.set_hexpand(true);
        let touch = gtk::GestureClick::builder().touch_only(true).build();
        touch.set_propagation_phase(gtk::PropagationPhase::Capture);
        touch.connect_pressed(|gesture, _, _, _| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
        });
        let touch_item = item.downgrade();
        let touch_list = list_weak.clone();
        touch.connect_released(move |_, presses, _, _| {
            let (Some(item), Some(list)) = (touch_item.upgrade(), touch_list.upgrade()) else {
                return;
            };
            let position = item.position();
            if presses == 1 && position != gtk::INVALID_LIST_POSITION {
                list.emit_by_name::<()>("activate", &[&position]);
            }
        });
        content.add_controller(touch);

        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        title.set_lines(2);
        title.add_css_class("heading");
        content.append(&title);

        let metadata = gtk::Label::new(None);
        metadata.set_xalign(0.0);
        metadata.set_ellipsize(gtk::pango::EllipsizeMode::End);
        metadata.add_css_class("dim-label");
        metadata.add_css_class("caption");
        content.append(&metadata);
        let read_toggle = gtk::Button::builder()
            .icon_name("mail-read-symbolic")
            .tooltip_text("Mark Read")
            .valign(gtk::Align::Center)
            .css_classes(["flat", "circular"])
            .build();
        read_toggle.set_action_target_value(Some(&0_i64.to_variant()));
        read_toggle.set_action_name(Some("win.mark-read"));
        read_toggle.update_property(&[gtk::accessible::Property::Label("Mark Read")]);
        row.append(&content);
        if mark_read_action {
            row.append(&read_toggle);
        }
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, item| {
        let item = item
            .downcast_ref::<gtk::ListItem>()
            .expect("factory item must be a GtkListItem");
        let entry = item
            .item()
            .and_downcast::<EntryObject>()
            .expect("inbox model must contain EntryObject values");
        let row = item
            .child()
            .and_downcast::<gtk::Box>()
            .expect("factory child must be a GtkBox");
        row.set_widget_name(&format!("article-{}", entry.entry().id));
        let content = row
            .first_child()
            .and_downcast::<gtk::Box>()
            .expect("first row child must be the content box");
        let title = content
            .first_child()
            .and_downcast::<gtk::Label>()
            .expect("first row child must be the title label");
        let metadata_label = title
            .next_sibling()
            .and_downcast::<gtk::Label>()
            .expect("second row child must be the metadata label");
        title.set_label(&entry.entry().title);
        metadata_label.set_label(&metadata(entry.entry()));
        if let Some(read_toggle) = content.next_sibling().and_downcast::<gtk::Button>() {
            let is_read = entry.entry().read;
            let label = if is_read { "Mark Unread" } else { "Mark Read" };
            read_toggle.set_icon_name(if is_read {
                "mail-unread-symbolic"
            } else {
                "mail-read-symbolic"
            });
            read_toggle.set_tooltip_text(Some(label));
            read_toggle.update_property(&[gtk::accessible::Property::Label(label)]);
            read_toggle.set_action_name(Some(if is_read {
                "win.mark-unread"
            } else {
                "win.mark-read"
            }));
            read_toggle.set_action_target_value(Some(&entry.entry().id.to_variant()));
        }
    });
    list.set_factory(Some(&factory));

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed({
        let list = list.clone();
        move |_, key, _, modifiers| {
            if !modifiers.is_empty() {
                return glib::Propagation::Proceed;
            }
            if (key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter)
                && list.has_focus()
                && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
                && selection.selected() != gtk::INVALID_LIST_POSITION
            {
                list.emit_by_name::<()>("activate", &[&selection.selected()]);
                return glib::Propagation::Stop;
            }
            let Some(direction) = cursor_direction(key) else {
                return glib::Propagation::Proceed;
            };
            if move_cursor(&list, direction) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        }
    });
    list.add_controller(keys);
    InboxModel { store, selection }
}

pub fn replace(model: &InboxModel, entries: Vec<Entry>) {
    let selected_id = model
        .selection
        .selected_item()
        .and_downcast::<EntryObject>()
        .map(|item| item.entry().id);
    for (index, entry) in entries.iter().enumerate() {
        let index = index as u32;
        if entry_at(model, index).is_some_and(|current| current.id == entry.id) {
            if entry_at(model, index).as_ref() != Some(entry) {
                model
                    .store
                    .splice(index, 1, &[EntryObject::new(entry.clone())]);
            }
            continue;
        }
        let existing = ((index + 1)..model.store.n_items()).find(|position| {
            entry_at(model, *position).is_some_and(|current| current.id == entry.id)
        });
        if let Some(existing) = existing {
            let item = model
                .store
                .item(existing)
                .and_downcast::<EntryObject>()
                .expect("existing entry must be an EntryObject");
            model.store.remove(existing);
            if item.entry() == entry {
                model.store.insert(index, &item);
            } else {
                model.store.insert(index, &EntryObject::new(entry.clone()));
            }
        } else {
            model.store.insert(index, &EntryObject::new(entry.clone()));
        }
    }
    while model.store.n_items() > entries.len() as u32 {
        model.store.remove(entries.len() as u32);
    }
    if let Some(id) = selected_id {
        let position = (0..model.store.n_items())
            .find(|position| entry_at(model, *position).is_some_and(|entry| entry.id == id));
        model
            .selection
            .set_selected(position.unwrap_or(gtk::INVALID_LIST_POSITION));
    }
}

pub fn update_entry(model: &InboxModel, entry: Entry) -> bool {
    let Some(position) = (0..model.store.n_items())
        .find(|position| entry_at(model, *position).is_some_and(|item| item.id == entry.id))
    else {
        return false;
    };
    model.store.splice(position, 1, &[EntryObject::new(entry)]);
    true
}

pub fn entry_at(model: &InboxModel, position: u32) -> Option<Entry> {
    model
        .store
        .item(position)
        .and_downcast::<EntryObject>()
        .map(|object| object.entry().clone())
}

pub fn selected_from_list(list: &gtk::ListView) -> Option<Entry> {
    let selection = list.model()?.downcast::<gtk::SingleSelection>().ok()?;
    selection
        .item(selection.selected())?
        .downcast::<EntryObject>()
        .ok()
        .map(|item| item.entry().clone())
}

pub fn selected_id(list: &gtk::ListView) -> Option<i64> {
    selected_from_list(list).map(|entry| entry.id)
}

pub fn select_id(list: &gtk::ListView, entry_id: i64) -> bool {
    let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>() else {
        return false;
    };
    let Some(position) = position_of_id(list, entry_id) else {
        return false;
    };
    selection.set_selected(position);
    true
}

pub fn cursor_direction(key: gtk::gdk::Key) -> Option<i32> {
    match key {
        gtk::gdk::Key::j | gtk::gdk::Key::Down => Some(1),
        gtk::gdk::Key::k | gtk::gdk::Key::Up => Some(-1),
        _ => None,
    }
}

pub fn move_cursor(list: &gtk::ListView, direction: i32) -> bool {
    let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>() else {
        return false;
    };
    let Some(position) = adjacent_position(selection.selected(), selection.n_items(), direction)
    else {
        return false;
    };
    list.grab_focus();
    selection.set_selected(position);
    list.scroll_to(position, gtk::ListScrollFlags::FOCUS, None);
    true
}

pub fn position_of_id(list: &gtk::ListView, entry_id: i64) -> Option<u32> {
    let selection = list.model().and_downcast::<gtk::SingleSelection>()?;
    (0..selection.n_items()).find(|position| {
        selection
            .item(*position)
            .and_downcast::<EntryObject>()
            .is_some_and(|item| item.entry().id == entry_id)
    })
}

pub fn entry_by_id(model: &InboxModel, entry_id: i64) -> Option<Entry> {
    (0..model.store.n_items())
        .find_map(|position| entry_at(model, position).filter(|entry| entry.id == entry_id))
}

pub fn remove_entry(model: &InboxModel, entry_id: i64) -> Option<Entry> {
    let position = (0..model.store.n_items())
        .find(|position| entry_at(model, *position).is_some_and(|entry| entry.id == entry_id))?;
    let entry = entry_at(model, position)?;
    model.store.remove(position);
    Some(entry)
}

pub fn insert_entry(model: &InboxModel, entry: Entry) {
    let position = (0..model.store.n_items())
        .find(|position| {
            entry_at(model, *position).is_some_and(|existing| {
                (existing.published_at_ms, existing.id) < (entry.published_at_ms, entry.id)
            })
        })
        .unwrap_or_else(|| model.store.n_items());
    model.store.insert(position, &EntryObject::new(entry));
}

fn adjacent_position(selected: u32, count: u32, direction: i32) -> Option<u32> {
    if count == 0 {
        return None;
    }
    if selected == gtk::INVALID_LIST_POSITION {
        return Some(if direction < 0 { count - 1 } else { 0 });
    }
    Some(if direction < 0 {
        selected.saturating_sub(1)
    } else {
        selected.saturating_add(1).min(count - 1)
    })
}

fn metadata(entry: &Entry) -> String {
    let mut parts = vec![entry.feed_title.clone()];
    if entry.read {
        parts.push("Read".into());
    }
    if let Some(author) = entry.author.as_deref() {
        parts.push(author.to_owned());
    }
    if entry.reading_minutes > 0 {
        parts.push(format!("{} min", entry.reading_minutes));
    }
    if let Some(age) = age(entry.published_at_ms) {
        parts.push(age);
    }
    if let Some(state) = entry.delivery_state {
        parts.push(match state {
            DeliveryState::Queued | DeliveryState::Sending => "Karakeep queued".into(),
            DeliveryState::Saved => "In Karakeep".into(),
            DeliveryState::NeedsAttention => "Karakeep needs attention".into(),
        });
    }
    parts.join(" · ")
}

fn age(published_at_ms: i64) -> Option<String> {
    if published_at_ms <= 0 {
        return None;
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let seconds = now_ms.saturating_sub(published_at_ms) / 1_000;
    Some(match seconds {
        0..=59 => "now".into(),
        60..=3_599 => format!("{}m", seconds / 60),
        3_600..=86_399 => format!("{}h", seconds / 3_600),
        86_400..=604_799 => format!("{}d", seconds / 86_400),
        _ => format!("{}w", seconds / 604_800),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_entry(id: i64) -> Entry {
        Entry {
            id,
            account_id: 1,
            feed_id: 2,
            feed_title: "Example".into(),
            category_title: String::new(),
            title: format!("Story {id}"),
            url: format!("https://example.com/{id}"),
            author: None,
            published_at_ms: 0,
            html: String::new(),
            read: false,
            starred: false,
            reading_minutes: 3,
            delivery_state: None,
            delivery_error: None,
        }
    }

    #[test]
    fn sync_toast_reports_membership_and_content_changes_not_total_unread() {
        let before = vec![example_entry(1), example_entry(2), example_entry(3)];
        let mut changed = example_entry(2);
        changed.title = "Updated story".into();
        let after = vec![example_entry(4), changed, example_entry(3)];
        let summary = InboxChanges::between(&before, &after);
        assert_eq!(
            summary,
            InboxChanges {
                added: 1,
                updated: 1
            }
        );
        assert_eq!(
            summary.toast_message(),
            Some("1 new article · 1 updated".into())
        );
        assert_eq!(InboxChanges::between(&after, &after).toast_message(), None);
    }

    #[test]
    fn sync_toast_ignores_articles_leaving_the_inbox() {
        let before = vec![example_entry(1)];
        let changes = InboxChanges::between(&before, &[]);

        assert_eq!(changes.toast_message(), None);
    }

    #[test]
    fn sync_toast_pluralizes_arrivals() {
        let summary = InboxChanges::between(&[], &[example_entry(1), example_entry(2)]);
        assert_eq!(summary.toast_message(), Some("2 new articles".into()));
    }

    #[test]
    fn replacing_entries_keeps_the_keyboard_cursor_on_the_same_article() {
        let store = gio::ListStore::new::<EntryObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        selection.set_autoselect(false);
        let model = InboxModel { store, selection };
        replace(&model, vec![example_entry(2), example_entry(1)]);
        model.selection.set_selected(1);

        replace(
            &model,
            vec![example_entry(3), example_entry(2), example_entry(1)],
        );
        let selected = model
            .selection
            .selected_item()
            .and_downcast::<EntryObject>()
            .expect("the selected article remains in the model");
        assert_eq!(selected.entry().id, 1);
        assert_eq!(model.selection.selected(), 2);
    }

    #[test]
    fn metadata_contains_the_useful_compact_fields() {
        let entry = Entry {
            id: 1,
            account_id: 1,
            feed_id: 2,
            feed_title: "Example".into(),
            category_title: String::new(),
            title: "Story".into(),
            url: "https://example.com/story".into(),
            author: Some("Ada".into()),
            published_at_ms: 0,
            html: String::new(),
            read: false,
            starred: false,
            reading_minutes: 3,
            delivery_state: None,
            delivery_error: None,
        };
        assert_eq!(metadata(&entry), "Example · Ada · 3 min");
    }

    #[test]
    fn list_navigation_selects_without_wrapping_or_activating() {
        assert_eq!(adjacent_position(gtk::INVALID_LIST_POSITION, 3, 1), Some(0));
        assert_eq!(
            adjacent_position(gtk::INVALID_LIST_POSITION, 3, -1),
            Some(2)
        );
        assert_eq!(adjacent_position(0, 3, -1), Some(0));
        assert_eq!(adjacent_position(1, 3, 1), Some(2));
        assert_eq!(adjacent_position(2, 3, 1), Some(2));
        assert_eq!(adjacent_position(0, 0, 1), None);
    }
}
