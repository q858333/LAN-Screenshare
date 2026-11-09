fn main() {
    let plist = format!(
        "{}/Info.plist",
        std::env::var("CARGO_MANIFEST_DIR").unwrap()
    );
    println!("cargo:rerun-if-changed=Info.plist");
    println!("cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{plist}");
    tauri_build::build()
}
