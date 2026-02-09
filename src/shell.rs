use std::env;

/// Represents the detected shell type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Cmd,
    Sh,
    Unknown,
}

impl ShellType {
    /// Returns a human-readable name for the shell
    pub fn name(&self) -> &'static str {
        match self {
            ShellType::Bash => "Bash",
            ShellType::Zsh => "Zsh",
            ShellType::Fish => "Fish",
            ShellType::PowerShell => "PowerShell",
            ShellType::Cmd => "CMD",
            ShellType::Sh => "sh",
            ShellType::Unknown => "shell",
        }
    }
}

/// Detects the current shell type based on environment variables
pub fn detect_shell() -> ShellType {
    #[cfg(unix)]
    {
        detect_unix_shell()
    }

    #[cfg(windows)]
    {
        detect_windows_shell()
    }

    #[cfg(not(any(unix, windows)))]
    {
        ShellType::Unknown
    }
}

#[cfg(unix)]
fn detect_unix_shell() -> ShellType {
    // First check $SHELL environment variable
    if let Ok(shell) = env::var("SHELL") {
        let shell_lower = shell.to_lowercase();
        if shell_lower.contains("zsh") {
            return ShellType::Zsh;
        } else if shell_lower.contains("bash") {
            return ShellType::Bash;
        } else if shell_lower.contains("fish") {
            return ShellType::Fish;
        } else if shell_lower.ends_with("/sh") {
            return ShellType::Sh;
        }
    }

    // Fallback: check parent process name via /proc on Linux
    #[cfg(target_os = "linux")]
    if let Ok(cmdline) = std::fs::read_to_string("/proc/$PPID/comm") {
        let name = cmdline.trim().to_lowercase();
        if name.contains("zsh") {
            return ShellType::Zsh;
        } else if name.contains("bash") {
            return ShellType::Bash;
        } else if name.contains("fish") {
            return ShellType::Fish;
        }
    }

    ShellType::Unknown
}

#[cfg(windows)]
fn detect_windows_shell() -> ShellType {
    // Check for PowerShell indicators
    // PSModulePath is set in PowerShell sessions
    if env::var("PSModulePath").is_ok() {
        return ShellType::PowerShell;
    }

    // Check COMSPEC for cmd.exe (default Windows shell)
    if let Ok(comspec) = env::var("COMSPEC") {
        if comspec.to_lowercase().contains("cmd.exe") {
            return ShellType::Cmd;
        }
    }

    // Check if running in Git Bash or similar
    if let Ok(shell) = env::var("SHELL") {
        let shell_lower = shell.to_lowercase();
        if shell_lower.contains("bash") {
            return ShellType::Bash;
        } else if shell_lower.contains("zsh") {
            return ShellType::Zsh;
        }
    }

    // Default to CMD on Windows if nothing else matches
    ShellType::Cmd
}

/// Returns the OS name for display
pub fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Unix"
    }
}
