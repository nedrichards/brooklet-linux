use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

use adw::{gio, prelude::*};
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

pub fn configure(list: &gtk::ListView, open_id: Rc<Cell<Option<i64>>>) -> InboxModel {
    configure_with_action(list, true, open_id)
}

pub fn configure_with_action(
    list: &gtk::ListView,
    mark_read_action: bool,
    open_id: Rc<Cell<Option<i64>>>,
) -> InboxModel {
    let store = gio::ListStore::new::<EntryObject>();
    let selection = gtk::SingleSelection::new(Some(store.clone()));
    selection.set_autoselect(false);
    selection.set_can_unselect(true);
    list.set_model(Some(&selection));
    list.add_css_class("article-list");
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

        // GTK's row double-click handler grabs row focus after emitting
        // activate. Opening the reader transfers focus away from the list;
        // grabbing it back can scroll the inbox to its old keyboard cursor.
        // Claim the double click here before it bubbles to that handler.
        let double_click = gtk::GestureClick::builder().button(1).build();
        let click_item = item.downgrade();
        let click_list = list_weak.clone();
        double_click.connect_pressed(move |gesture, presses, _, _| {
            if presses != 2 {
                return;
            }
            let (Some(item), Some(list)) = (click_item.upgrade(), click_list.upgrade()) else {
                return;
            };
            let position = item.position();
            if position != gtk::INVALID_LIST_POSITION {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                list.emit_by_name::<()>("activate", &[&position]);
            }
        });
        content.add_controller(double_click);

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
    factory.connect_bind(move |_, item| {
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
        title.set_label(&entry.entry().title);
        set_open_marker(&row, entry.entry(), open_id.get());
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

    InboxModel { store, selection }
}

pub fn replace(model: &InboxModel, entries: Vec<Entry>) {
    let selected_id = model
        .selection
        .selected_item()
        .and_downcast::<EntryObject>()
        .map(|item| item.entry().id);
    // Compare borrowed bodies and retain existing objects. A single changed
    // interval avoids repeated scans, body clones, and items-changed emissions.
    let previous = (0..model.store.n_items())
        .map(|index| {
            model
                .store
                .item(index)
                .and_downcast::<EntryObject>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let prefix = previous
        .iter()
        .zip(&entries)
        .take_while(|(old, new)| old.entry() == *new)
        .count();
    let suffix = previous[prefix..]
        .iter()
        .rev()
        .zip(entries[prefix..].iter().rev())
        .take_while(|(old, new)| old.entry() == *new)
        .count();
    let old_end = previous.len() - suffix;
    let new_end = entries.len() - suffix;
    let selected_position =
        selected_id.and_then(|id| entries.iter().position(|entry| entry.id == id));
    if prefix != old_end || prefix != new_end {
        let existing = previous[prefix..old_end]
            .iter()
            .map(|item| (item.entry().id, item))
            .collect::<HashMap<_, _>>();
        let replacements = entries
            .into_iter()
            .skip(prefix)
            .take(new_end - prefix)
            .map(|entry| {
                if let Some(item) = existing
                    .get(&entry.id)
                    .filter(|item| item.entry() == &entry)
                {
                    (*item).clone()
                } else {
                    EntryObject::new(entry)
                }
            })
            .collect::<Vec<_>>();
        model
            .store
            .splice(prefix as u32, (old_end - prefix) as u32, &replacements);
    }
    if selected_id.is_some() {
        model.selection.set_selected(
            selected_position.map_or(gtk::INVALID_LIST_POSITION, |position| position as u32),
        );
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

fn set_open_marker(row: &gtk::Box, entry: &Entry, open_id: Option<i64>) {
    let is_open = open_id == Some(entry.id);
    if is_open {
        row.add_css_class("open-article");
    } else {
        row.remove_css_class("open-article");
    }
    let metadata_label = row
        .first_child()
        .and_downcast::<gtk::Box>()
        .and_then(|content| content.first_child())
        .and_then(|title| title.next_sibling())
        .and_downcast::<gtk::Label>()
        .expect("article row must contain a metadata label");
    let description = if is_open {
        format!("Open in reader · {}", metadata(entry))
    } else {
        metadata(entry)
    };
    metadata_label.set_label(&description);
}

pub fn refresh_open_marker(list: &gtk::ListView, open_id: Option<i64>) {
    fn visit(widget: &gtk::Widget, list: &gtk::ListView, open_id: Option<i64>) {
        if widget.has_css_class("article-row") {
            if let Some(entry_id) = widget
                .widget_name()
                .strip_prefix("article-")
                .and_then(|id| id.parse::<i64>().ok())
                && let Some(position) = position_of_id(list, entry_id)
                && let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>()
                && let Some(entry) = selection.item(position).and_downcast::<EntryObject>()
                && let Ok(row) = widget.clone().downcast::<gtk::Box>()
            {
                set_open_marker(&row, entry.entry(), open_id);
            }
            return;
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            visit(&widget, list, open_id);
            child = widget.next_sibling();
        }
    }
    visit(list.upcast_ref(), list, open_id);
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

pub fn select_next_unread(list: &gtk::ListView, pending: &HashSet<i64>) {
    let Some(selection) = list.model().and_downcast::<gtk::SingleSelection>() else {
        return;
    };
    let selected = selection.selected();
    if selected == gtk::INVALID_LIST_POSITION {
        return;
    }
    let count = selection.n_items();
    let next = ((selected + 1)..count)
        .chain((0..selected).rev())
        .find(|position| {
            selection
                .item(*position)
                .and_downcast::<EntryObject>()
                .is_some_and(|item| !item.entry().read && !pending.contains(&item.entry().id))
        });
    if let Some(next) = next {
        selection.set_selected(next);
        if list.is_mapped() {
            list.scroll_to(next, gtk::ListScrollFlags::FOCUS, None);
        }
    } else {
        selection.set_selected(gtk::INVALID_LIST_POSITION);
    }
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
            content_revision: 0,
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
    fn list_selection_survives_refresh_and_skips_pending_dismissals() {
        gtk::init().expect("GTK is available for the selection test");
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
        let mut navigation_entry = selected.shared_entry();
        assert!(std::sync::Arc::ptr_eq(
            &navigation_entry,
            &selected.shared_entry()
        ));
        std::sync::Arc::make_mut(&mut navigation_entry).read = true;
        assert!(
            !selected.entry().read,
            "navigation state must not mutate the list snapshot"
        );

        let changes = Rc::new(Cell::new(0));
        model.store.connect_items_changed({
            let changes = changes.clone();
            move |_, _, _, _| changes.set(changes.get() + 1)
        });
        replace(
            &model,
            vec![example_entry(3), example_entry(2), example_entry(1)],
        );
        assert_eq!(changes.get(), 0, "unchanged refresh emits no model changes");
        let retained = model.store.item(1).unwrap();
        replace(
            &model,
            vec![example_entry(2), example_entry(4), example_entry(1)],
        );
        assert_eq!(changes.get(), 1, "mixed changes are delivered atomically");
        assert_eq!(model.store.item(0).unwrap(), retained);
        assert_eq!(model.selection.selected(), 2);
        replace(&model, vec![example_entry(4)]);
        assert_eq!(model.selection.selected(), gtk::INVALID_LIST_POSITION);
        replace(&model, Vec::new());
        assert_eq!(model.store.n_items(), 0);

        let list = gtk::ListView::new(None::<gtk::SelectionModel>, None::<gtk::ListItemFactory>);
        let model = configure(&list, Rc::new(Cell::new(None)));
        replace(
            &model,
            vec![example_entry(1), example_entry(2), example_entry(3)],
        );
        model.selection.set_selected(0);

        select_next_unread(&list, &HashSet::from([2]));

        assert_eq!(selected_id(&list), Some(3));
        assert!(!entry_at(&model, 0).expect("first row remains").read);
    }

    #[test]
    #[ignore = "manual synthetic performance probe; run alone with --nocapture"]
    fn large_list_refresh_performance() {
        gtk::init().unwrap();
        let store = gio::ListStore::new::<EntryObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        let model = InboxModel { store, selection };
        let entries = (0..5000)
            .map(|id| Entry {
                html: "x".repeat(16 * 1024),
                content_revision: 0,
                ..example_entry(id)
            })
            .collect::<Vec<_>>();
        replace(&model, entries.clone());
        for scenario in ["unchanged", "prepend", "reverse", "remove-half"] {
            let mut incoming = entries.clone();
            match scenario {
                "prepend" => incoming.insert(0, example_entry(5001)),
                "reverse" => incoming.reverse(),
                "remove-half" => incoming.retain(|entry| entry.id % 2 == 0),
                _ => {}
            }
            let start = std::time::Instant::now();
            replace(&model, incoming);
            eprintln!(
                "5000 articles, 16 KiB bodies, {scenario}: {:?}",
                start.elapsed()
            );
            replace(&model, entries.clone());
        }
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
            content_revision: 0,
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
