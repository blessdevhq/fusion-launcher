fn main() {
    use std::path::Path;

    // Rebuild when compile-time built-in scraper credentials change so an
    // incremental build picks up new values instead of a cached option_env!.
    println!("cargo:rerun-if-env-changed=FUSION_LAUNCHER_STEAMGRIDDB_KEY");
    println!("cargo:rerun-if-env-changed=RETROHYDRA_STEAMGRIDDB_KEY");
    println!("cargo:rerun-if-env-changed=FUSION_LAUNCHER_SCREENSCRAPER_DEVID");
    println!("cargo:rerun-if-env-changed=RETROHYDRA_SCREENSCRAPER_DEVID");
    println!("cargo:rerun-if-env-changed=FUSION_LAUNCHER_SCREENSCRAPER_DEVPASSWORD");
    println!("cargo:rerun-if-env-changed=RETROHYDRA_SCREENSCRAPER_DEVPASSWORD");

    // Tauri embeds the Windows app icon into the executable resource at build time.
    // Make icon/config edits relink the dev binary instead of reusing a stale exe.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=tauri.conf.json");

    for platform_config in [
        "tauri.windows.conf.json",
        "tauri.macos.conf.json",
        "tauri.linux.conf.json",
    ] {
        if Path::new(platform_config).exists() {
            println!("cargo:rerun-if-changed={platform_config}");
        }
    }

    tauri_build::build()
}
