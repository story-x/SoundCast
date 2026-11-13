use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let is_msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
        if is_msvc {
            embed_manifest(new_manifest("SoundCast.App"))
                .expect("unable to embed Windows manifest");
        }

        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "SoundCast");
        res.set("FileDescription", "SoundCast - 按显示器自动切换应用音频");
        res.compile().expect("unable to compile Windows icon resource");
    }

    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into());
    slint_build::compile_with_config("ui/settings.slint", config)
        .expect("unable to compile Slint UI");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=assets/icon.png");
    println!("cargo:rerun-if-changed=assets/logo.png");
    println!("cargo:rerun-if-changed=ui/settings.slint");
}
