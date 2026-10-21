fn main() {
    // `embed_migrations!` reads `migrations/` at compile time, so rebuild when a
    // migration is added or changed.
    println!("cargo:rerun-if-changed=migrations");

    // Windows reads an executable's icon (taskbar, Explorer, the installer's
    // shortcuts) from a resource compiled into it, unlike the window icon set at
    // runtime. A missing resource compiler only costs the icon, not the build.
    println!("cargo:rerun-if-changed=assets/komasabi.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/komasabi.ico");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed the Windows icon: {error}");
        }
    }
}
