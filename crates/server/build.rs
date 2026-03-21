fn main() {
    println!("cargo:rerun-if-changed=../../web/dist/");

    // rust-embed requires the folder to exist at compile time, even if empty.
    // Create a stub so the build succeeds without web assets.
    let dist = std::path::Path::new("../../web/dist");
    if !dist.exists() {
        let _ = std::fs::create_dir_all(dist);
        println!(
            "cargo:warning=web/dist/ not found — the jit-server binary will not include \
             embedded web assets. Run 'cd web && npm run build' first, then rebuild."
        );
    } else if !dist.join("index.html").exists() {
        println!(
            "cargo:warning=web/dist/index.html not found — the jit-server binary will not \
             include usable web assets. Run 'cd web && npm run build' first, then rebuild."
        );
    }
}
