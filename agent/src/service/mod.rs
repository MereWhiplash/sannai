use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

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
        Platform::MacOS => PathBuf::from(&home).join("Library/LaunchAgents/dev.sannai.agent.plist"),
        Platform::Linux => PathBuf::from(&home).join(".config/systemd/user/sannai.service"),
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

/// Install service file to a specific path (testable).
pub fn install_service_to(platform: Platform, bin_path: &str, path: &Path) -> Result<()> {
    if path.exists() {
        bail!("Service already installed at {}. Run `sannai uninstall` first.", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = match platform {
        Platform::MacOS => generate_launchd_plist(bin_path),
        Platform::Linux => generate_systemd_unit(bin_path),
    };
    std::fs::write(path, content)?;
    Ok(())
}

/// Install service and load it.
pub fn install_service() -> Result<()> {
    let platform = detect_platform();
    let bin_path = std::env::current_exe()?.to_string_lossy().to_string();
    let path = service_file_path(platform);

    install_service_to(platform, &bin_path, &path)?;

    match platform {
        Platform::MacOS => {
            let status = std::process::Command::new("launchctl")
                .args(["load", &path.to_string_lossy()])
                .status()?;
            if !status.success() {
                bail!("Failed to load launchd service");
            }
            println!("Installed and loaded launchd service.");
            println!("Sannai will start automatically on login.");
        }
        Platform::Linux => {
            std::process::Command::new("systemctl").args(["--user", "daemon-reload"]).status()?;
            let status = std::process::Command::new("systemctl")
                .args(["--user", "enable", "--now", "sannai"])
                .status()?;
            if !status.success() {
                bail!("Failed to enable systemd service");
            }
            println!("Installed and enabled systemd user service.");
            println!("Sannai will start automatically on login.");
        }
    }

    println!("Service file: {}", path.display());
    Ok(())
}

/// Remove service file (testable).
pub fn uninstall_service_from(path: &Path) -> Result<()> {
    if !path.exists() {
        println!("No service installed.");
        return Ok(());
    }
    std::fs::remove_file(path)?;
    Ok(())
}

/// Remove the data directory.
pub fn purge_data_dir(data_dir: &Path) -> Result<()> {
    if data_dir.exists() {
        std::fs::remove_dir_all(data_dir)?;
        println!("Removed data directory: {}", data_dir.display());
    }
    Ok(())
}

/// Uninstall service, optionally purge data.
pub fn uninstall_service(purge: bool) -> Result<()> {
    let platform = detect_platform();
    let path = service_file_path(platform);

    if path.exists() {
        match platform {
            Platform::MacOS => {
                std::process::Command::new("launchctl")
                    .args(["unload", &path.to_string_lossy()])
                    .status()
                    .ok();
            }
            Platform::Linux => {
                std::process::Command::new("systemctl")
                    .args(["--user", "disable", "--now", "sannai"])
                    .status()
                    .ok();
            }
        }
    }

    uninstall_service_from(&path)?;

    if purge {
        let data_dir = crate::daemon::data_dir();
        purge_data_dir(&data_dir)?;
    }

    println!("Sannai service uninstalled.");
    Ok(())
}

/// Check if service is installed at a specific path (testable).
pub fn is_service_installed_at(path: &Path) -> bool {
    path.exists()
}

/// Check if service is installed.
pub fn is_service_installed() -> bool {
    let platform = detect_platform();
    is_service_installed_at(&service_file_path(platform))
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

    #[test]
    fn test_install_writes_service_file() {
        let dir = tempfile::tempdir().unwrap();
        let service_path = dir.path().join("test.plist");
        let bin_path = "/usr/local/bin/sannai";

        install_service_to(Platform::MacOS, bin_path, &service_path).unwrap();

        assert!(service_path.exists());
        let content = std::fs::read_to_string(&service_path).unwrap();
        assert!(content.contains("dev.sannai.agent"));
    }

    #[test]
    fn test_install_refuses_if_already_installed() {
        let dir = tempfile::tempdir().unwrap();
        let service_path = dir.path().join("test.plist");
        let bin_path = "/usr/local/bin/sannai";

        install_service_to(Platform::MacOS, bin_path, &service_path).unwrap();
        let result = install_service_to(Platform::MacOS, bin_path, &service_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_uninstall_removes_service_file() {
        let dir = tempfile::tempdir().unwrap();
        let service_path = dir.path().join("test.plist");
        let bin_path = "/usr/local/bin/sannai";

        install_service_to(Platform::MacOS, bin_path, &service_path).unwrap();
        assert!(service_path.exists());

        uninstall_service_from(&service_path).unwrap();
        assert!(!service_path.exists());
    }

    #[test]
    fn test_uninstall_noop_if_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let service_path = dir.path().join("nonexistent.plist");

        let result = uninstall_service_from(&service_path);
        assert!(result.is_ok());
    }

    #[test]
    fn test_purge_removes_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("store.db"), "test").unwrap();

        purge_data_dir(&data_dir).unwrap();
        assert!(!data_dir.exists());
    }

    #[test]
    fn test_is_service_installed() {
        let dir = tempfile::tempdir().unwrap();

        let not_installed = dir.path().join("nope.plist");
        assert!(!is_service_installed_at(&not_installed));

        let installed = dir.path().join("yes.plist");
        std::fs::write(&installed, "test").unwrap();
        assert!(is_service_installed_at(&installed));
    }
}
