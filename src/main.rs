mod application;
mod keyboard;
mod logging;
mod ui;

fn main() -> adw::glib::ExitCode {
    logging::init();
    if std::env::args_os().any(|argument| argument == "--search-test") {
        return match application::search_test() {
            Ok(()) => adw::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Brooklet search regression failed: {error}");
                adw::glib::ExitCode::FAILURE
            }
        };
    }
    if std::env::args_os().any(|argument| argument == "--keyboard-test") {
        return match application::keyboard_test() {
            Ok(()) => adw::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Brooklet keyboard regression failed: {error}");
                adw::glib::ExitCode::FAILURE
            }
        };
    }
    if std::env::args_os().any(|argument| argument == "--reader-test") {
        return match application::reader_test() {
            Ok(()) => adw::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Brooklet reader regression failed: {error}");
                adw::glib::ExitCode::FAILURE
            }
        };
    }
    if std::env::args_os().any(|argument| argument == "--smoke-test") {
        return match application::smoke_test() {
            Ok(()) => adw::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Brooklet launch smoke test failed: {error}");
                adw::glib::ExitCode::FAILURE
            }
        };
    }
    application::BrookletApplication::run()
}
