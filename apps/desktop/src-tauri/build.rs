fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(&[
                "get_snapshot", "discover_models", "run_task", "cancel_task",
                "openai_key_configured", "save_openai_key", "delete_openai_key",
                "inspect_foreground", "capture_foreground", "discover_vision_models", "analyze_capture",
                "enter_character_map_digit",
                "local_files_root", "preview_local_text", "copy_local_text",
            ])),
    )
    .expect("failed to build the Tauri application");
}
