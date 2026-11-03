//! Windows: embeds the application icon and the version resource (what Explorer's
//! Properties > Details and Task Manager show). gpui already embeds the manifest
//! (DPI awareness, common controls) through its `windows-manifest` feature, so the
//! resource script written here carries none; a second RT_MANIFEST would collide.
//!
//! The icon is `assets/icons/weep/weep.ico`, rendered from the logo by
//! `tools/windows/make_icon.py`. The version comes from the crate's.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        windows_resources();
    }
}

fn windows_resources() {
    use std::path::PathBuf;

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest_dir.join("assets/icons/weep/weep.ico");
    println!("cargo:rerun-if-changed={}", icon.display());

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let number = |key: &str| std::env::var(key).unwrap().parse::<u16>().unwrap_or(0);
    let (major, minor, patch) = (
        number("CARGO_PKG_VERSION_MAJOR"),
        number("CARGO_PKG_VERSION_MINOR"),
        number("CARGO_PKG_VERSION_PATCH"),
    );
    let icon_path = icon.display().to_string().replace('\\', "\\\\");

    let rc = format!(
        r#"#pragma code_page(65001)
1 ICON "{icon_path}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "WEEP"
            VALUE "FileDescription", "WEEP"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "weep"
            VALUE "LegalCopyright", "Copyright (c) 2026 ragtux. MIT License."
            VALUE "OriginalFilename", "weep.exe"
            VALUE "ProductName", "WEEP"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#
    );

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("weep.rc");
    std::fs::write(&out, rc).unwrap();
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
