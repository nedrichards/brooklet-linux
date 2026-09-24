use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-changed=resources/ui/window.blp");
    println!("cargo:rerun-if-changed=resources/ui/setup-dialog.blp");
    println!("cargo:rerun-if-changed=resources/ui/shortcuts-dialog.blp");
    println!("cargo:rerun-if-changed=resources/style.css");

    if env::var_os("CARGO_FEATURE_GUI").is_none() {
        return;
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    compile_blueprint(&out_dir, "window");
    compile_blueprint(&out_dir, "setup-dialog");
    compile_blueprint(&out_dir, "shortcuts-dialog");

    let manifest = out_dir.join("brooklet.gresource.xml");
    fs::write(
        &manifest,
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gresources>
  <gresource prefix="/com/nedrichards/brooklet">
    <file alias="ui/window.ui">window.ui</file>
    <file alias="ui/setup-dialog.ui">setup-dialog.ui</file>
    <file alias="ui/shortcuts-dialog.ui">shortcuts-dialog.ui</file>
    <file alias="style.css">style.css</file>
  </gresource>
</gresources>
"#,
    )
    .expect("write generated GResource manifest");

    run(
        Command::new("glib-compile-resources")
            .arg("--target")
            .arg(out_dir.join("brooklet.gresource"))
            .arg("--sourcedir")
            .arg(&out_dir)
            .arg("--sourcedir")
            .arg("resources")
            .arg(&manifest),
        "compile GResources",
    );
}

fn compile_blueprint(out_dir: &Path, name: &str) -> PathBuf {
    let output = out_dir.join(format!("{name}.ui"));
    run(
        Command::new("blueprint-compiler")
            .args(["compile", "--output"])
            .arg(&output)
            .arg(format!("resources/ui/{name}.blp")),
        "compile Blueprint UI",
    );
    output
}

fn run(command: &mut Command, description: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("failed to {description}: {error}");
    });
    assert!(status.success(), "failed to {description}");
}
