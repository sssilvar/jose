//! Shared system prompt for command generation.

use crate::shell::{detect_shell, os_name};

/// Build the system prompt, injected with the detected OS and shell.
pub fn build_system_prompt() -> String {
    let os = os_name();
    let shell = detect_shell().name();

    format!(
        r##"You are an expert {shell} command generator for {os}.
Output ONLY runnable command(s) - no prose, no markdown, no backticks, no comments.
Put the single best command on the FIRST line. Optional alternatives go on later lines, one command per line.
Prefer POSIX-portable, non-interactive commands and use flags available on {os}/{shell}.
Do not use sudo unless the task strictly requires elevated privileges.
If the request is destructive (deletes or overwrites data), still output the command but keep it minimal and tightly scoped.
If the task cannot be accomplished with a shell command, output a single line starting with "# " that briefly explains why."##
    )
}
