#[cfg(target_os = "windows")]
fn main() {
    use std::{borrow::Cow, env, fs, path::PathBuf};

    println!("cargo:rerun-if-changed=assets/app.manifest");
    println!("cargo:rerun-if-changed=build.rs");

    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let icon_path = output.join("table-arranger-control.ico");
    fs::write(&icon_path, make_icon()).expect("write generated application icon");
    let release_manifest = include_str!("assets/app.manifest");
    let manifest = if env::var("PROFILE").as_deref() == Ok("release") {
        Cow::Borrowed(release_manifest)
    } else {
        Cow::Owned(release_manifest.replace("requireAdministrator", "asInvoker"))
    };

    let mut resources = winres::WindowsResource::new();
    resources
        .set_icon(icon_path.to_str().expect("UTF-8 icon path"))
        .set_manifest(&manifest)
        .set("ProductName", "Table Arranger Control")
        .set("FileDescription", "Table Arranger Control")
        .set("LegalCopyright", "Copyright (c) 2026")
        .set("OriginalFilename", "table-arranger-control.exe");
    resources.compile().expect("compile Windows resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}

#[cfg(target_os = "windows")]
fn make_icon() -> Vec<u8> {
    const WIDTH: usize = 32;
    const HEIGHT: usize = 32;
    const PIXELS: usize = WIDTH * HEIGHT * 4;
    const MASK: usize = HEIGHT * 4;
    const IMAGE_BYTES: usize = 40 + PIXELS + MASK;
    let width_u8 = u8::try_from(WIDTH).expect("icon width fits u8");
    let height_u8 = u8::try_from(HEIGHT).expect("icon height fits u8");
    let image_bytes_u32 = u32::try_from(IMAGE_BYTES).expect("icon image size fits u32");
    let width_i32 = i32::try_from(WIDTH).expect("icon width fits i32");
    let doubled_height_i32 = i32::try_from(HEIGHT * 2).expect("icon height fits i32");
    let pixels_u32 = u32::try_from(PIXELS).expect("icon pixel size fits u32");

    let mut icon = Vec::with_capacity(22 + IMAGE_BYTES);
    icon.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    icon.extend_from_slice(&[width_u8, height_u8, 0, 0]);
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&32_u16.to_le_bytes());
    icon.extend_from_slice(&image_bytes_u32.to_le_bytes());
    icon.extend_from_slice(&22_u32.to_le_bytes());

    icon.extend_from_slice(&40_u32.to_le_bytes());
    icon.extend_from_slice(&width_i32.to_le_bytes());
    icon.extend_from_slice(&doubled_height_i32.to_le_bytes());
    icon.extend_from_slice(&1_u16.to_le_bytes());
    icon.extend_from_slice(&32_u16.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());
    icon.extend_from_slice(&pixels_u32.to_le_bytes());
    icon.extend_from_slice(&0_i32.to_le_bytes());
    icon.extend_from_slice(&0_i32.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());
    icon.extend_from_slice(&0_u32.to_le_bytes());

    for y in (0..HEIGHT).rev() {
        for x in 0..WIDTH {
            let border = x < 2 || y < 2 || x >= WIDTH - 2 || y >= HEIGHT - 2;
            let grid = (x == 15 || x == 16 || y == 15 || y == 16)
                && (4..28).contains(&x)
                && (4..28).contains(&y);
            let table = (4..28).contains(&x) && (4..28).contains(&y);
            let (r, g, b) = if border {
                (17, 24, 39)
            } else if grid {
                (244, 248, 255)
            } else if table {
                (22, 163, 74)
            } else {
                (34, 197, 94)
            };
            icon.extend_from_slice(&[b, g, r, 255]);
        }
    }
    icon.resize(icon.len() + MASK, 0);
    icon
}
