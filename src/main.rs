mod application;
mod logging;
mod ui;

fn main() -> adw::glib::ExitCode {
    logging::init();
    if std::env::args_os().any(|argument| argument == "--smoke-test") {
        return match application::smoke_test() {
            Ok(()) => adw::glib::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Brooklet launch smoke test failed: {error}");
                adw::glib::ExitCode::FAILURE
            }
        };
    }
    match application::BrookletApplication::new() {
        Ok(application) => application.run(),
        Err(error) => {
            eprintln!("Brooklet could not start: {error}");
            adw::glib::ExitCode::FAILURE
        }
    }
}
