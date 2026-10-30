//! Copies the built web UI (`web/dist`) into `dist/`, which `tauri.conf.json` embeds. Without a build the app still
//! compiles, with a placeholder page that says how to build the UI, as the daemon does.
use std::path::Path;
use std::{env, fs, io};

const PLACEHOLDER: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>Fleet Manager</title></head><body>\
<div id=\"root\"><p>Fleet Manager's web UI is not built into this binary. Run <code>pnpm --dir web build</code> and rebuild.</p></div></body></html>";

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn main() {
    let here =
        Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).to_path_buf();
    let dist = here.join("../../web/dist");
    let out = here.join("dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let _ = fs::remove_dir_all(&out);
    if dist.join("index.html").is_file() {
        copy_dir(&dist, &out).expect("copy web/dist");
    } else {
        fs::create_dir_all(&out).expect("create dist");
        fs::write(out.join("index.html"), PLACEHOLDER).expect("write placeholder");
        println!("cargo:warning=web/dist is not built; embedding the placeholder page");
    }
    tauri_build::build();
}
