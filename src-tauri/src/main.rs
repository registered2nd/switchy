// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // On Linux, set a WebKit environment variable to work around DMA-BUF rendering problems:
    // on some systems (e.g. Debian 13.2, Nvidia GPUs) WebKitGTK's DMA-BUF renderer causes a white or black screen.
    // See: https://github.com/tauri-apps/tauri/issues/9394
    #[cfg(target_os = "linux")]
    {
        if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    switchy_lib::run();
}
