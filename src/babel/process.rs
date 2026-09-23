use std::{
    io::{Read, Write},
    os::unix::process::CommandExt,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use rustix::process::{Pid, Signal, kill_process_group};

use super::execution::BabelExecutor;

pub(super) fn execute_external(
    executor: BabelExecutor,
    source: &str,
    root: &Path,
) -> Result<String, String> {
    execute_external_with_timeout(executor, source, root, Duration::from_secs(15))
}

fn execute_external_with_timeout(
    executor: BabelExecutor,
    source: &str,
    root: &Path,
    timeout: Duration,
) -> Result<String, String> {
    const MAX_OUTPUT: usize = 1024 * 1024;
    let (program, args): (&str, &[&str]) = match executor {
        BabelExecutor::Python => ("python3", &["-"]),
        BabelExecutor::Shell => ("/bin/sh", &[]),
        BabelExecutor::Bash => ("/bin/bash", &[]),
        _ => unreachable!("only external languages use a process"),
    };
    let mut child = Command::new(program)
        .args(args)
        .current_dir(if root.as_os_str().is_empty() { Path::new(".") } else { root })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|error| format!("Could not start {program}: {error}. Check that it is installed and available on PATH"))?;
    let input = child.stdin.take().expect("piped stdin");
    let source = source.to_owned();
    let writer = std::thread::spawn(move || {
        let mut input = input;
        input.write_all(source.as_bytes())
    });
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = std::thread::spawn(move || read_capped(stdout, MAX_OUTPUT));
    let stderr_reader = std::thread::spawn(move || read_capped(stderr, MAX_OUTPUT));
    let started = Instant::now();
    let mut status = None;
    loop {
        if status.is_none() {
            status = child
                .try_wait()
                .map_err(|error| format!("Could not wait for {program}: {error}"))?;
        }
        if status.is_some()
            && writer.is_finished()
            && stdout_reader.is_finished()
            && stderr_reader.is_finished()
        {
            break;
        }
        if started.elapsed() >= timeout {
            if let Some(group) = i32::try_from(child.id()).ok().and_then(Pid::from_raw) {
                let _ = kill_process_group(group, Signal::KILL);
            }
            if status.is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(format!(
                "{program} timed out after {} seconds",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = writer.join();
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| "Could not read stdout".to_owned())?
        .map_err(|error| format!("Could not read stdout: {error}"))?;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| "Could not read stderr".to_owned())?
        .map_err(|error| format!("Could not read stderr: {error}"))?;
    if stdout_truncated || stderr_truncated {
        return Err(format!("{program} output exceeds 1 MiB"));
    }
    let status = status.expect("process exit checked above");
    if !status.success() {
        let detail = if stderr.is_empty() { &stdout } else { &stderr };
        let detail = String::from_utf8_lossy(detail);
        return Err(format!("{program} exited with {status}: {}", detail.trim()));
    }
    String::from_utf8(stdout).map_err(|_| format!("{program} produced non-UTF-8 output"))
}

fn read_capped(mut pipe: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut output = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let count = pipe.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let kept = count.min(limit.saturating_sub(output.len()));
        output.extend_from_slice(&buffer[..kept]);
        truncated |= kept < count;
    }
    Ok((output, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_includes_output_pipes_held_by_background_children() {
        let error = execute_external_with_timeout(
            BabelExecutor::Shell,
            "sleep 2 &\n",
            Path::new("."),
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }
}
