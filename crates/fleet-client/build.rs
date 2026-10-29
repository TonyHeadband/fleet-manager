//! Windows only: embeds the icon and version info in the executables. A no-op on other platforms.
fn main() {
    #[cfg(windows)]
    {
        println!("cargo:rerun-if-changed=../../packaging/fleet-manager.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/fleet-manager.ico")
            .set("ProductName", "Fleet Manager")
            .set("FileDescription", "Fleet Manager attach");
        res.compile().expect("embed the Windows resource");
    }
}
