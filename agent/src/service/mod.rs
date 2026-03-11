use anyhow::{bail, Result};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Platform {
    MacOS,
    Linux,
}

pub fn detect_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::MacOS
    } else {
        Platform::Linux
    }
}

pub fn service_file_path(platform: Platform) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    match platform {
        Platform::MacOS => PathBuf::from(&home)
            .join("Library/LaunchAgents/dev.sannai.agent.plist"),
        Platform::Linux => PathBuf::from(&home)
            .join(".config/systemd/user/sannai.service"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_platform() {
        let platform = detect_platform();
        #[cfg(target_os = "macos")]
        assert_eq!(platform, Platform::MacOS);
        #[cfg(target_os = "linux")]
        assert_eq!(platform, Platform::Linux);
    }

    #[test]
    fn test_service_file_path() {
        let path = service_file_path(Platform::MacOS);
        assert!(path.to_string_lossy().contains("LaunchAgents"));
        assert!(path.to_string_lossy().contains("dev.sannai.agent.plist"));

        let path = service_file_path(Platform::Linux);
        assert!(path.to_string_lossy().contains("systemd/user"));
        assert!(path.to_string_lossy().contains("sannai.service"));
    }
}
