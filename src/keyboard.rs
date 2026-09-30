//! Shared definitions for dispatch, application accelerators and shortcut help.
use gtk::gdk::{Key, ModifierType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Action(&'static str),
    Next,
    Previous,
    First,
    Last,
    PageDown,
    PageUp,
    Open,
    Read,
    Save,
    Browser,
    Copy,
    Undo,
    NextArticle,
    PreviousArticle,
    Back,
    NextPane,
    PreviousPane,
    Destination(&'static str),
    Menu,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Window,
    Articles,
    List,
    Reader,
}

pub struct Binding {
    pub context: Context,
    pub section: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub keys: &'static [&'static str],
    pub command: Command,
}

// Mark literals for gettext extraction; translate them when constructing the UI.
const fn message(text: &'static str) -> &'static str {
    text
}

macro_rules! binding {
    ($section:expr, $title:expr, $subtitle:expr, [$($key:literal),+], $command:expr, $context:expr) => {
        Binding { context: $context, section: $section, title: $title, subtitle: $subtitle,
            keys: &[$($key),+], command: $command }
    };
}

pub const BINDINGS: &[Binding] = &[
    binding!(
        message("General"),
        message("Sync articles"),
        "",
        ["<Control>r"],
        Command::Action("app.sync"),
        Context::Window
    ),
    binding!(
        message("General"),
        message("Search Library"),
        "",
        ["<Control>f"],
        Command::Action("app.search"),
        Context::Window
    ),
    binding!(
        message("General"),
        message("Preferences"),
        "",
        ["<Control>comma"],
        Command::Action("win.preferences"),
        Context::Window
    ),
    binding!(
        message("General"),
        message("Show keyboard shortcuts"),
        message("Anywhere in Brooklet"),
        ["F1", "<Control>question", "<Control><Shift>slash"],
        Command::Action("app.show-shortcuts"),
        Context::Window
    ),
    binding!(
        message("General"),
        message("Close window"),
        "",
        ["<Control>w"],
        Command::Action("win.close"),
        Context::Window
    ),
    binding!(
        message("General"),
        message("Quit Brooklet"),
        "",
        ["<Control>q"],
        Command::Action("app.quit"),
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Go back"),
        message("Close a popup or return to the source list"),
        ["Escape", "<Alt>Left"],
        Command::Back,
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Focus next pane"),
        message("Article list and reader in wide layouts"),
        ["F6"],
        Command::NextPane,
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Focus previous pane"),
        message("Article list and reader in wide layouts"),
        ["<Shift>F6"],
        Command::PreviousPane,
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Show Inbox"),
        "",
        ["<Control>1"],
        Command::Destination("inbox"),
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Show Saved"),
        "",
        ["<Control>2"],
        Command::Destination("saved"),
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Show Library"),
        "",
        ["<Control>3"],
        Command::Destination("library"),
        Context::Window
    ),
    binding!(
        message("Navigation"),
        message("Open main menu"),
        "",
        ["F10"],
        Command::Menu,
        Context::Window
    ),
    binding!(
        message("Article lists and reader"),
        message("Move down"),
        message("Select next article in lists; scroll in the reader"),
        ["Down", "j"],
        Command::Next,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Move up"),
        message("Select previous article in lists; scroll in the reader"),
        ["Up", "k"],
        Command::Previous,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Go to beginning"),
        message("First article in lists; top of the reader"),
        ["Home"],
        Command::First,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Go to end"),
        message("Last article in lists; bottom of the reader"),
        ["End"],
        Command::Last,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Move one page down"),
        message("Selection in lists; scrolling in the reader"),
        ["Page_Down"],
        Command::PageDown,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Move one page up"),
        message("Selection in lists; scrolling in the reader"),
        ["Page_Up"],
        Command::PageUp,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Toggle read or unread"),
        message("Selected or open article; marking the reader unread returns to its list"),
        ["r"],
        Command::Read,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Save or unsave article"),
        message("Selected or open article"),
        ["s", "<Control>d"],
        Command::Save,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Open in browser"),
        message("Selected or open article"),
        ["b"],
        Command::Browser,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Copy article link"),
        message("Selected or open article"),
        ["<Control><Shift>c"],
        Command::Copy,
        Context::Articles
    ),
    binding!(
        message("Article lists and reader"),
        message("Undo Mark Read"),
        message("Outside text fields"),
        ["u", "<Control>z"],
        Command::Undo,
        Context::Articles
    ),
    binding!(
        message("Article lists"),
        message("Open selected article"),
        message("Selection alone does not mark it read"),
        ["Return", "KP_Enter"],
        Command::Open,
        Context::List
    ),
    binding!(
        message("Reader"),
        message("Scroll one page down"),
        message("Outside buttons, links and text selection"),
        ["space"],
        Command::PageDown,
        Context::Reader
    ),
    binding!(
        message("Reader"),
        message("Scroll one page up"),
        message("Outside buttons, links and text selection"),
        ["<Shift>space"],
        Command::PageUp,
        Context::Reader
    ),
    binding!(
        message("Reader"),
        message("Open next article"),
        message("From the source collection"),
        ["n"],
        Command::NextArticle,
        Context::Reader
    ),
    binding!(
        message("Reader"),
        message("Open previous article"),
        message("From the source collection"),
        ["p"],
        Command::PreviousArticle,
        Context::Reader
    ),
];

pub fn modifiers(modifiers: ModifierType) -> ModifierType {
    modifiers
        & (ModifierType::SHIFT_MASK
            | ModifierType::CONTROL_MASK
            | ModifierType::ALT_MASK
            | ModifierType::SUPER_MASK
            | ModifierType::HYPER_MASK
            | ModifierType::META_MASK)
}

pub fn parsed_bindings() -> Vec<(Key, ModifierType, &'static Binding)> {
    BINDINGS
        .iter()
        .flat_map(|binding| {
            binding.keys.iter().map(move |accelerator| {
                let (key, modifiers) = gtk::accelerator_parse(*accelerator)
                    .unwrap_or_else(|| panic!("Invalid Brooklet shortcut: {accelerator}"));
                (key, modifiers, binding)
            })
        })
        .collect()
}

pub fn populate_dialog(dialog: &adw::ShortcutsDialog) {
    let mut section_name = "";
    let mut section = None;
    for binding in BINDINGS {
        if section_name != binding.section {
            section_name = binding.section;
            let next = adw::ShortcutsSection::new(Some(&adw::glib::dgettext(
                Some("brooklet"),
                section_name,
            )));
            dialog.add(next.clone());
            section = Some(next);
        }
        for accelerator in binding.keys {
            let item = adw::ShortcutsItem::new(
                &adw::glib::dgettext(Some("brooklet"), binding.title),
                accelerator,
            );
            if !binding.subtitle.is_empty() {
                item.set_subtitle(&adw::glib::dgettext(Some("brooklet"), binding.subtitle));
            }
            section.as_ref().unwrap().add(item);
        }
    }
}

fn hint(binding: &Binding) -> String {
    let (key, modifiers) = gtk::accelerator_parse(binding.keys[0]).unwrap();
    gtk::accelerator_get_label(key, modifiers).to_string()
}

pub fn action_hint(action: &str) -> String {
    let command =
        match action {
            "win.toggle-star" => Command::Save,
            "win.open-browser" => Command::Browser,
            "win.copy-link" => Command::Copy,
            "win.next-article" => Command::NextArticle,
            "win.previous-article" => Command::PreviousArticle,
            _ => return BINDINGS
                .iter()
                .find(|binding| matches!(binding.command, Command::Action(name) if name == action))
                .map(hint)
                .unwrap_or_default(),
        };
    BINDINGS
        .iter()
        .find(|binding| binding.command == command)
        .map(hint)
        .unwrap_or_default()
}
