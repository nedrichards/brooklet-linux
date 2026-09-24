pub const APP_ID: &str = match option_env!("BROOKLET_APP_ID") {
    Some(value) => value,
    None => "com.nedrichards.brooklet.Devel",
};
pub const APP_NAME: &str = "Brooklet";
