// build.rs — embed Windows EXE icon
// Converts assets/app_icon.png to a multi-size ICO and embeds it into the EXE.

fn main() {
    // skip on non-Windows targets
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // PNG → ICO conversion
    let png_path = "assets/app_icon.png";
    let ico_path = "assets/app_icon.ico";

    println!("cargo:rerun-if-changed={png_path}");

    let img = image::open(png_path)
        .expect("failed to open assets/app_icon.png");

    let sizes: &[u32] = &[16, 32, 48, 256];
    let mut icon_dir = ico::IconDir::new(ico::ResourceType::Icon);

    for &sz in sizes {
        let resized = img.resize_exact(sz, sz, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let ico_img = ico::IconImage::from_rgba_data(sz, sz, resized.into_raw());
        icon_dir.add_entry(ico::IconDirEntry::encode(&ico_img)
            .expect("failed to encode ICO entry"));
    }

    let ico_file = std::fs::File::create(ico_path)
        .expect("failed to create assets/app_icon.ico");
    icon_dir.write(ico_file)
        .expect("failed to write ICO");

    // embed the icon into the EXE via winresource
    let mut res = winresource::WindowsResource::new();
    res.set_icon(ico_path);
    res.compile().expect("winresource compile failed");
}
