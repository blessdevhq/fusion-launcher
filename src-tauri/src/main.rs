fn main() {
    let package_smoke = std::env::var("FUSION_LAUNCHER_PACKAGE_SMOKE")
        .or_else(|_| std::env::var("RETROHYDRA_PACKAGE_SMOKE"));
    if package_smoke.as_deref() == Ok("1") {
        if let Err(error) = fusion_launcher_lib::run_package_smoke() {
            eprintln!("Fusion Launcher package smoke failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    fusion_launcher_lib::run()
}
