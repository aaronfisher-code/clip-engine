fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if let Ok(library) = pkg_config::Config::new()
        .atleast_version("0.35.0")
        .probe("mpv")
    {
        for path in library.link_paths {
            println!("cargo:rustc-link-search=native={}", path.display());
        }
        for lib in library.libs {
            println!("cargo:rustc-link-lib={lib}");
        }
        return;
    }
    println!("cargo:rustc-link-lib=mpv");
    if let Ok(directory) = std::env::var("MPV_LIB_DIR") {
        println!("cargo:rustc-link-search=native={directory}");
    }
}
