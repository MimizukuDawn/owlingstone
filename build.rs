//! Windows: puts the owl icon and the version into Owlingstone.exe (what Explorer and the taskbar show).

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico").set("ProductName", "Owlingstone").set("FileDescription", "Owlingstone");
        if let Err(e) = res.compile() {
            panic!("could not add the icon to the exe: {e}");
        }
    }
}
