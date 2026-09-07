fn main() {
    println!("cargo:rerun-if-changed=assets/icons/neovim-gpui.ico");
    println!("cargo:rerun-if-changed=assets/icons/neovim-gpui.rc");

    #[cfg(target_os = "windows")]
    embed_resource::compile("assets/icons/neovim-gpui.rc", embed_resource::NONE)
        .manifest_optional()
        .expect("failed to embed the Windows application icon");
}
