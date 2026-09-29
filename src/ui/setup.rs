use std::{cell::RefCell, rc::Rc, sync::Arc};

use adw::prelude::*;
use brooklet::{controller::AppController, model::Account, setup::SetupRequest};

pub fn present(
    parent: &adw::ApplicationWindow,
    controller: Arc<AppController>,
    on_complete: impl Fn(Account) + 'static,
) {
    let builder = gtk::Builder::from_resource("/com/nedrichards/brooklet/ui/setup-dialog.ui");
    let dialog: adw::Dialog = object(&builder, "setup_dialog");
    let signals = super::signal_scope::SignalScope::for_dialog(&dialog);
    let server_entry: adw::EntryRow = object(&builder, "server_entry");
    let token_entry: adw::PasswordEntryRow = object(&builder, "token_entry");
    let connect_button: gtk::Button = object(&builder, "connect_button");
    let progress_box: gtk::Box = object(&builder, "progress_box");
    let error_revealer: gtk::Revealer = object(&builder, "error_revealer");
    let error_label: gtk::Label = object(&builder, "error_label");
    let form_content: gtk::Box = object(&builder, "form_content");
    let success_content: gtk::Box = object(&builder, "success_content");
    let success_title: gtk::Label = object(&builder, "success_title");
    let success_detail: gtk::Label = object(&builder, "success_detail");
    let continue_button: gtk::Button = object(&builder, "continue_button");
    let configured_account = Rc::new(RefCell::new(None::<Account>));
    let on_complete: Rc<dyn Fn(Account)> = Rc::new(on_complete);

    let update_submit = {
        let server_entry = server_entry.clone();
        let token_entry = token_entry.clone();
        let connect_button = connect_button.clone();
        Rc::new(move || {
            connect_button.set_sensitive(
                !server_entry.text().trim().is_empty() && !token_entry.text().trim().is_empty(),
            );
        })
    };
    signals.track(
        &server_entry,
        server_entry.connect_changed({
            let update_submit = update_submit.clone();
            move |_| update_submit()
        }),
    );
    signals.track(
        &token_entry,
        token_entry.connect_changed({
            let update_submit = update_submit.clone();
            move |_| update_submit()
        }),
    );
    update_submit();

    let submit: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let server_entry = server_entry.clone();
        let token_entry = token_entry.clone();
        let connect_button = connect_button.clone();
        let progress_box = progress_box.clone();
        let error_revealer = error_revealer.clone();
        let error_label = error_label.clone();
        let form_content = form_content.clone();
        let success_content = success_content.clone();
        let success_title = success_title.clone();
        let success_detail = success_detail.clone();
        let configured_account = configured_account.clone();
        move || {
            if !connect_button.is_sensitive() {
                return;
            }
            let request = SetupRequest {
                server_url: server_entry.text().to_string(),
                token: token_entry.text().to_string(),
            };

            token_entry.set_text("");
            server_entry.set_sensitive(false);
            token_entry.set_sensitive(false);
            connect_button.set_sensitive(false);
            progress_box.set_visible(true);
            error_revealer.set_reveal_child(false);
            dialog.set_can_close(false);

            controller.configure(request, {
                let dialog = dialog.clone();
                let server_entry = server_entry.clone();
                let token_entry = token_entry.clone();
                let connect_button = connect_button.clone();
                let progress_box = progress_box.clone();
                let error_revealer = error_revealer.clone();
                let error_label = error_label.clone();
                let form_content = form_content.clone();
                let success_content = success_content.clone();
                let success_title = success_title.clone();
                let success_detail = success_detail.clone();
                let configured_account = configured_account.clone();
                move |result| {
                    progress_box.set_visible(false);
                    dialog.set_can_close(true);
                    match result {
                        Ok(account) => {
                            success_title.set_label(&format!("Welcome, {}", account.username));
                            success_detail.set_label(&format!(
                                "Connected to {}\nMiniflux {}",
                                account.server_url, account.server_version
                            ));
                            *configured_account.borrow_mut() = Some(account);
                            form_content.set_visible(false);
                            success_content.set_visible(true);
                        }
                        Err(error) => {
                            error_label.set_label(&error.setup_message());
                            error_revealer.set_reveal_child(true);
                            server_entry.set_sensitive(true);
                            token_entry.set_sensitive(true);
                            connect_button.set_sensitive(false);
                            token_entry.grab_focus();
                        }
                    }
                }
            });
        }
    });

    signals.track(
        &connect_button,
        connect_button.connect_clicked({
            let submit = submit.clone();
            move |_| submit()
        }),
    );
    signals.track(
        &token_entry,
        token_entry.connect_entry_activated({
            let submit = submit.clone();
            move |_| submit()
        }),
    );
    signals.track(
        &server_entry,
        server_entry.connect_entry_activated({
            let token_entry = token_entry.clone();
            move |_| {
                token_entry.grab_focus();
            }
        }),
    );
    signals.track(
        &continue_button,
        continue_button.connect_clicked({
            let dialog = dialog.clone();
            let configured_account = configured_account.clone();
            move |_| {
                if let Some(account) = configured_account.borrow_mut().take() {
                    on_complete(account);
                    dialog.close();
                }
            }
        }),
    );

    dialog.present(Some(parent));
    server_entry.grab_focus();
}

fn object<T: adw::glib::object::IsA<adw::glib::Object> + Clone + 'static>(
    builder: &gtk::Builder,
    name: &str,
) -> T {
    builder
        .object(name)
        .unwrap_or_else(|| panic!("setup-dialog.ui must define {name}"))
}
