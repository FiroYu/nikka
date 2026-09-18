fn main() {
    // 自定义 manifest：在默认基础上补 PerMonitorV2 DPI 声明（见 app.manifest 注释）。
    tauri_build::try_build(
        tauri_build::Attributes::default()
            .windows_attributes(tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"))),
    )
    .expect("failed to run tauri-build");
}
