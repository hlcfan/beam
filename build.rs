fn main() {
    println!("cargo:rerun-if-env-changed=BEAM_UPDATE_PUBLIC_KEY");
    #[cfg(windows)]
    embed_windows_resources();
}

#[cfg(windows)]
fn embed_windows_resources() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.set("ProductName", "Beam");
    res.set("FileDescription", "Beam HTTP client");
    if let Err(err) = res.compile() {
        panic!("failed to compile Windows resources: {err}");
    }
}
