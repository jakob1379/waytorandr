use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::profile::Hook;

const HOOK_FAILURE_OUTPUT_LIMIT: usize = 240;

#[derive(Debug, Default)]
#[non_exhaustive]
pub struct HookResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub elapsed_secs: f64,
    pub stdout: String,
    pub stderr: String,
    pub phase: Option<String>,
    pub command: Option<String>,
}

pub(crate) fn run_hooks(hooks: &[Hook], phase: &str) -> Vec<HookResult> {
    let mut results = Vec::new();
    for hook in hooks {
        let result = execute_hook(hook, phase);
        results.push(result);
    }
    results
}

fn execute_hook(hook: &Hook, phase: &str) -> HookResult {
    let start = Instant::now();
    let timeout = Duration::from_secs(hook.timeout_secs);

    let mut cmd = Command::new(&hook.command);
    cmd.args(&hook.args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    match cmd.spawn() {
        Ok(mut child) => loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    let elapsed = start.elapsed();
                    return match child.wait_with_output() {
                        Ok(output) => hook_result_from_output(&output, elapsed, phase, hook),
                        Err(err) => failed_hook_result(
                            elapsed,
                            String::new(),
                            format!("Hook output collection failed: {err}"),
                            phase,
                            hook,
                        ),
                    };
                }
                Ok(None) => {
                    if start.elapsed() > timeout {
                        return timeout_hook_result(child, start.elapsed(), phase, hook);
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(err) => {
                    return failed_hook_result(
                        start.elapsed(),
                        String::new(),
                        format!("Hook wait failed: {err}"),
                        phase,
                        hook,
                    );
                }
            }
        },
        Err(err) => failed_hook_result(
            Duration::ZERO,
            String::new(),
            format!("Failed to spawn: {err}"),
            phase,
            hook,
        ),
    }
}

fn timeout_hook_result(
    mut child: Child,
    elapsed: Duration,
    phase: &str,
    hook: &Hook,
) -> HookResult {
    let mut stdout = String::new();
    let mut stderr = match child.kill() {
        Ok(()) => "Hook timed out".to_string(),
        Err(err) => format!("Hook timed out and could not be killed: {err}"),
    };

    match child.wait_with_output() {
        Ok(output) => {
            stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let output_stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            if !output_stderr.is_empty() {
                stderr = format!("{stderr}\n{output_stderr}");
            }
        }
        Err(err) => {
            stderr = format!("{stderr}\nHook output collection failed: {err}");
        }
    }

    failed_hook_result(elapsed, stdout, stderr, phase, hook)
}

fn failed_hook_result(
    elapsed: Duration,
    stdout: String,
    stderr: String,
    phase: &str,
    hook: &Hook,
) -> HookResult {
    HookResult {
        success: false,
        exit_code: None,
        elapsed_secs: elapsed.as_secs_f64(),
        stdout,
        stderr,
        phase: Some(phase.to_string()),
        command: Some(hook.command.clone()),
    }
}

fn hook_result_from_output(
    output: &Output,
    elapsed: Duration,
    phase: &str,
    hook: &Hook,
) -> HookResult {
    HookResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        elapsed_secs: elapsed.as_secs_f64(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        phase: Some(phase.to_string()),
        command: Some(hook.command.clone()),
    }
}

pub(crate) fn format_hook_failure(result: &HookResult) -> String {
    let phase = result.phase.as_deref().unwrap_or("hook");
    let command = result.command.as_deref().unwrap_or("<unknown>");
    let exit = result
        .exit_code
        .map_or_else(String::new, |code| format!(" (exit code {code})"));
    if !result.stderr.trim().is_empty() {
        let stderr = bounded_hook_output(&result.stderr);
        format!("{phase} hook '{command}' failed{exit}: {stderr}")
    } else if !result.stdout.trim().is_empty() {
        let stdout = bounded_hook_output(&result.stdout);
        format!("{phase} hook '{command}' failed{exit}: stdout: {stdout}")
    } else {
        format!("{phase} hook '{command}' failed{exit}")
    }
}

fn bounded_hook_output(output: &str) -> String {
    let output = output.trim();
    let mut chars = output.chars();
    let bounded: String = chars.by_ref().take(HOOK_FAILURE_OUTPUT_LIMIT).collect();
    if chars.next().is_some() {
        format!("{bounded}...")
    } else {
        bounded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_hook(script: &str) -> Hook {
        let mut hook = Hook::new("sh");
        hook.args = vec!["-c".to_string(), script.to_string()];
        hook
    }

    #[test]
    fn run_hooks_captures_command_output() {
        let hook = shell_hook("printf 'out'; printf 'err' >&2");

        let results = run_hooks(&[hook], "pre-apply");

        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(result.success);
        assert_eq!(result.stdout, "out");
        assert_eq!(result.stderr, "err");
        assert_eq!(result.phase.as_deref(), Some("pre-apply"));
        assert_eq!(result.command.as_deref(), Some("sh"));
    }

    #[test]
    fn run_hooks_reports_timeout_failures() {
        let mut hook = shell_hook("sleep 1");
        hook.timeout_secs = 0;

        let result = run_hooks(&[hook], "post-apply").remove(0);

        assert!(!result.success);
        assert!(result.stderr.contains("Hook timed out"));
        assert_eq!(result.phase.as_deref(), Some("post-apply"));
        assert_eq!(
            format_hook_failure(&result),
            format!("post-apply hook 'sh' failed: {}", result.stderr.trim())
        );
    }

    #[test]
    fn format_hook_failure_includes_exit_code_and_stdout_when_stderr_is_empty() {
        let result = HookResult {
            success: false,
            exit_code: Some(7),
            stdout: "useful stdout\n".to_string(),
            stderr: String::new(),
            phase: Some("pre-apply".to_string()),
            command: Some("notify".to_string()),
            ..HookResult::default()
        };

        assert_eq!(
            format_hook_failure(&result),
            "pre-apply hook 'notify' failed (exit code 7): stdout: useful stdout"
        );
    }

    #[test]
    fn format_hook_failure_prefers_stderr_and_includes_exit_code() {
        let result = HookResult {
            success: false,
            exit_code: Some(3),
            stdout: "ignored stdout".to_string(),
            stderr: "useful stderr\n".to_string(),
            phase: Some("post-apply".to_string()),
            command: Some("notify".to_string()),
            ..HookResult::default()
        };

        assert_eq!(
            format_hook_failure(&result),
            "post-apply hook 'notify' failed (exit code 3): useful stderr"
        );
    }

    #[test]
    fn format_hook_failure_bounds_long_stdout() {
        let result = HookResult {
            success: false,
            exit_code: Some(1),
            stdout: "x".repeat(HOOK_FAILURE_OUTPUT_LIMIT + 1),
            stderr: String::new(),
            phase: Some("failure".to_string()),
            command: Some("notify".to_string()),
            ..HookResult::default()
        };
        let message = format_hook_failure(&result);

        assert!(message.ends_with("..."));
        assert!(message.len() < HOOK_FAILURE_OUTPUT_LIMIT + 80);
    }
}
