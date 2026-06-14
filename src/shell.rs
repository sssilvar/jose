use std::env;
use std::path::PathBuf;

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
    if let Some(name) = parent_process_name() {
        let name = name.to_lowercase();
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

/// Read the parent process's command name from `/proc` (Linux only).
///
/// The parent PID comes from field 4 of `/proc/self/stat`; the command name is
/// then read from `/proc/<ppid>/comm`.
#[cfg(target_os = "linux")]
fn parent_process_name() -> Option<String> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    // Fields after the (possibly space/paren-containing) comm field start after
    // the last ')'. Layout from there: state ppid ...
    let after_comm = stat.rsplit_once(')')?.1;
    let ppid = after_comm.split_whitespace().nth(1)?;
    let comm = std::fs::read_to_string(format!("/proc/{ppid}/comm")).ok()?;
    Some(comm.trim().to_string())
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

/// A snapshot of the environment the generated commands will run in.
///
/// Everything here is gathered cheaply (compile-time constants, env vars, and a
/// couple of small file reads) — no subprocesses are spawned, so it adds no
/// meaningful latency to a query.
pub struct SystemInfo {
    pub os: &'static str,
    pub os_version: Option<String>,
    pub arch: &'static str,
    pub shell: ShellType,
    /// Flavor of the userland tools: "GNU" (Linux) or "BSD" (macOS/*BSD).
    /// Flag syntax for sed/find/date/stat/xargs differs between them.
    pub coreutils: &'static str,
    pub package_managers: Vec<&'static str>,
}

impl SystemInfo {
    pub fn gather() -> Self {
        Self {
            os: os_name(),
            os_version: os_version(),
            arch: env::consts::ARCH,
            shell: detect_shell(),
            coreutils: coreutils_flavor(),
            package_managers: detect_package_managers(),
        }
    }
}

/// "BSD" userland on macOS and the BSDs; "GNU" elsewhere (Linux). This is the
/// single most important hint for command quality — BSD and GNU differ on
/// common flags (e.g. `sed -i ''` vs `sed -i`, `date -r` vs `date -d`).
fn coreutils_flavor() -> &'static str {
    if cfg!(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )) {
        "BSD"
    } else {
        "GNU"
    }
}

#[cfg(target_os = "macos")]
fn os_version() -> Option<String> {
    // Parse ProductVersion out of the system plist without a plist crate.
    let txt = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let key = txt.find("ProductVersion")?;
    let open = txt[key..].find("<string>")? + key + "<string>".len();
    let close = txt[open..].find("</string>")? + open;
    Some(txt[open..close].trim().to_string())
}

#[cfg(target_os = "linux")]
fn os_version() -> Option<String> {
    let txt = std::fs::read_to_string("/etc/os-release").ok()?;
    txt.lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_string())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn os_version() -> Option<String> {
    None
}

/// Detect installed package managers by scanning PATH for known binaries.
/// No subprocess is spawned — we only stat candidate paths.
fn detect_package_managers() -> Vec<&'static str> {
    const CANDIDATES: &[&str] = &[
        "brew", "port", "apt", "dnf", "yum", "pacman", "zypper", "apk", "nix-env", "snap",
        "flatpak", "winget", "choco", "scoop",
    ];

    let path = match env::var_os("PATH") {
        Some(p) => p,
        None => return Vec::new(),
    };
    let dirs: Vec<PathBuf> = env::split_paths(&path).collect();

    CANDIDATES
        .iter()
        .copied()
        .filter(|name| {
            dirs.iter().any(|dir| {
                if dir.join(name).is_file() {
                    return true;
                }
                #[cfg(windows)]
                {
                    dir.join(format!("{name}.exe")).is_file()
                        || dir.join(format!("{name}.cmd")).is_file()
                }
                #[cfg(not(windows))]
                {
                    false
                }
            })
        })
        .collect()
}
