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

pub fn generate_launchd_plist(bin_path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>dev.sannai.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin_path}</string>
        <string>start</string>
        <string>--foreground</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{home}/Library/Logs/sannai.log</string>
    <key>StandardErrorPath</key>
    <string>{home}/Library/Logs/sannai.log</string>
</dict>
</plist>
"#
    )
}

pub fn generate_systemd_unit(bin_path: &str) -> String {
    format!(
        r#"[Unit]
Description=Sannai AI coding session capture daemon
After=default.target

[Service]
Type=simple
ExecStart={bin_path} start --foreground
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
"#
    )
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

    #[test]
    fn test_generate_launchd_plist() {
        let plist = generate_launchd_plist("/usr/local/bin/sannai");
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("dev.sannai.agent"));
        assert!(plist.contains("/usr/local/bin/sannai"));
        assert!(plist.contains("start"));
        assert!(plist.contains("--foreground"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("sannai.log"));
    }

    #[test]
    fn test_generate_systemd_unit() {
        let unit = generate_systemd_unit("/usr/local/bin/sannai");
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("/usr/local/bin/sannai start --foreground"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }
}
