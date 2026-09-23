use crate::error::AppError;
use auto_launch::{AutoLaunch, AutoLaunchBuilder};

/// Get the .app bundle path on macOS
/// Converts `/path/to/Switchy.app/Contents/MacOS/Switchy` to `/path/to/Switchy.app`
#[cfg(target_os = "macos")]
fn get_macos_app_bundle_path(exe_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let path_str = exe_path.to_string_lossy();
    // Look for the .app/Contents/MacOS/ pattern
    if let Some(app_pos) = path_str.find(".app/Contents/MacOS/") {
        let app_bundle_end = app_pos + 4; // end of ".app"
        Some(std::path::PathBuf::from(&path_str[..app_bundle_end]))
    } else {
        None
    }
}

/// Initialise the AutoLaunch instance
fn get_auto_launch() -> Result<AutoLaunch, AppError> {
    let app_name = "Switchy";
    let exe_path = std::env::current_exe()
        .map_err(|e| AppError::Message(format!("Failed to get the application path: {e}")))?;

    // macOS needs the .app bundle path; otherwise the AppleScript login item opens a terminal
    #[cfg(target_os = "macos")]
    let app_path = get_macos_app_bundle_path(&exe_path).unwrap_or(exe_path);

    #[cfg(not(target_os = "macos"))]
    let app_path = exe_path;

    // AutoLaunchBuilder hides the platform differences
    // macOS: AppleScript (the default), which needs the .app bundle path
    // Windows/Linux: registry / XDG autostart
    let auto_launch = AutoLaunchBuilder::new()
        .set_app_name(app_name)
        .set_app_path(&app_path.to_string_lossy())
        .build()
        .map_err(|e| AppError::Message(format!("Failed to create AutoLaunch: {e}")))?;

    Ok(auto_launch)
}

/// Enable launch at login
pub fn enable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .enable()
        .map_err(|e| AppError::Message(format!("Failed to enable launch at login: {e}")))?;
    log::info!("Launch at login enabled");
    Ok(())
}

/// Disable launch at login
pub fn disable_auto_launch() -> Result<(), AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .disable()
        .map_err(|e| AppError::Message(format!("Failed to disable launch at login: {e}")))?;
    log::info!("Launch at login disabled");
    Ok(())
}

/// Check whether launch at login is enabled
pub fn is_auto_launch_enabled() -> Result<bool, AppError> {
    let auto_launch = get_auto_launch()?;
    auto_launch
        .is_enabled()
        .map_err(|e| AppError::Message(format!("Failed to check launch-at-login status: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_valid() {
        let exe_path = std::path::Path::new("/Applications/Switchy.app/Contents/MacOS/Switchy");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(
            result,
            Some(std::path::PathBuf::from("/Applications/Switchy.app"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_with_spaces() {
        let exe_path =
            std::path::Path::new("/Users/test/My Apps/Switchy.app/Contents/MacOS/Switchy");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(
            result,
            Some(std::path::PathBuf::from("/Users/test/My Apps/Switchy.app"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_not_in_bundle() {
        let exe_path = std::path::Path::new("/usr/local/bin/switchy");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(result, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_get_macos_app_bundle_path_dev_build() {
        // In development the path is usually not inside an .app bundle
        let exe_path = std::path::Path::new("/Users/dev/project/target/debug/switchy");
        let result = get_macos_app_bundle_path(exe_path);
        assert_eq!(result, None);
    }
}
