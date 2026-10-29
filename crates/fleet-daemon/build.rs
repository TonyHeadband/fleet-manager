//! Copies the built web UI (`web/dist`) next to the compiled code so it can be embedded in the binary.
//! Without a build the daemon still compiles, serving a placeholder page that says how to build the UI.
use std::path::Path;
use std::{env, fs, io};

const PLACEHOLDER: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>fleet-manager</title></head><body>\
<div id=\"root\"><p>fleet-manager's web UI is not built into this binary. Run <code>./build.sh</code> and rebuild.</p></div></body></html>";

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

/// Windows only: the icon and version info of the executables.
#[cfg(windows)]
fn embed_windows_resource() {
    println!("cargo:rerun-if-changed=../../packaging/fleet-manager.ico");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/fleet-manager.ico")
        .set("ProductName", "fleet-manager")
        .set("FileDescription", "fleet-manager daemon");
    res.compile().expect("embed the Windows resource");
}

fn main() {
    #[cfg(windows)]
    embed_windows_resource();
    let out = Path::new(&env::var("OUT_DIR").expect("OUT_DIR")).join("web");
    let dist = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"))
        .join("../../web/dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed=build.rs");
    let _ = fs::remove_dir_all(&out);
    if dist.join("index.html").is_file() {
        copy_dir(&dist, &out).expect("copy web/dist");
    } else {
        fs::create_dir_all(&out).expect("create OUT_DIR/web");
        fs::write(out.join("index.html"), PLACEHOLDER).expect("write placeholder");
        println!(
            "cargo:warning=web/dist is not built; embedding the placeholder page (the web UI returns with remote access)"
        );
    }
}
