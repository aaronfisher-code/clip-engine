fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/clip-engine.ico");
    embed_windows_icon();
    link_mpv();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/binaries");
    }
}

fn embed_windows_icon() {
    #[cfg(windows)]
    {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/clip-engine.ico");
        resource.set("ProductName", "Dabs Clip Engine");
        resource.set("FileDescription", "Dabs Clip Engine");
        resource.set("CompanyName", "Dab");
        resource
            .compile()
            .expect("embed the Dabs Clip Engine Windows icon");
    }
}

fn link_mpv() {
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
