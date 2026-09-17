fn main() {
    tauri_build::build();

    // Tauri emits link-arg-bins, omitting the library test harness. It
    // imports TaskDialogIndirect and needs the same Common Controls v6
    // manifest as the application. Reuse Tauri's generated GNU resource.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
    {
        let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
        let resource = out.join("libresource.a");
        assert!(resource.is_file(), "Tauri GNU resource was not generated");
        // Bins already receive it via Tauri's link-arg-bins; this covers
        // the library test harness too (duplicate static inclusion is
        // harmless -- verified the export binary still runs and passes).
        println!("cargo:rustc-link-arg={}", resource.display());
    }
}
