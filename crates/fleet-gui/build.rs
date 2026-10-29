//! Windows only: embeds the icon and version info in the executables. A no-op on other platforms.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/fleet-manager.ico")
            .set("ProductName", "fleet-manager")
            .set("FileDescription", "fleet-manager");
        res.compile().expect("embed the Windows resource");
    }
}
