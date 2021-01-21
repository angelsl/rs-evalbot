use std::io::Cursor;
use std::os::unix::process::ExitStatusExt;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Buf, BufMut, BytesMut};
use log::{debug, error};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::Command;
use tokio::time;

use crate::{ExecBackend, UnixSocketBackend};

fn strsig(sig: i32) -> &'static str {
    match sig {
        1 => "Hangup",
        2 => "Interrupt",
        3 => "Quit",
        4 => "Illegal instruction",
        5 => "Trace/breakpoint trap",
        6 => "Aborted",
        7 => "Bus error",
        8 => "Floating point exception",
        9 => "Killed",
        10 => "User defined signal 1",
        11 => "Segmentation fault",
        12 => "User defined signal 2",
        13 => "Broken pipe",
        14 => "Alarm clock",
        15 => "Terminated",
        16 => "Stack fault",
        17 => "Child exited",
        18 => "Continued",
        19 => "Stopped (signal)",
        20 => "Stopped",
        21 => "Stopped (tty input)",
        22 => "Stopped (tty output)",
        23 => "Urgent I/O condition",
        24 => "CPU time limit exceeded",
        25 => "File size limit exceeded",
        26 => "Virtual timer expired",
        27 => "Profiling timer expired",
        28 => "Window changed",
        29 => "I/O possible",
        30 => "Power failure",
        31 => "Bad system call",
        _ => "Unknown signal",
    }
}

fn strsigabbrev(sig: i32) -> &'static str {
    match sig {
        1 => "SIGHUP",
        2 => "SIGINT",
        3 => "SIGQUIT",
        4 => "SIGILL",
        5 => "SIGTRAP",
        6 => "SIGABRT",
        7 => "SIGBUS",
        8 => "SIGFPE",
        9 => "SIGKILL",
        10 => "SIGUSR1",
        11 => "SIGSEGV",
        12 => "SIGUSR2",
        13 => "SIGPIPE",
        14 => "SIGALRM",
        15 => "SIGTERM",
        16 => "SIGSTKFLT",
        17 => "SIGCHLD",
        18 => "SIGCONT",
        19 => "SIGSTOP",
        20 => "SIGTSTP",
        21 => "SIGTTIN",
        22 => "SIGTTOU",
        23 => "SIGURG",
        24 => "SIGXCPU",
        25 => "SIGXFSZ",
        26 => "SIGVTALRM",
        27 => "SIGPROF",
        28 => "SIGWINCH",
        29 => "SIGPOLL",
        30 => "SIGPWR",
        31 => "SIGSYS",
        _ => "(unknown)",
    }
}

pub async fn exec<'a, T>(
    lang: Arc<ExecBackend>,
    timeout: Option<usize>,
    code: T,
) -> Result<String, String>
where
    T: AsRef<[u8]> + 'a,
{
    let timeout_arg = format!(
        "{}{}",
        lang.timeout_prefix
            .as_ref()
            .map(String::as_str)
            .unwrap_or(""),
        timeout.unwrap_or(0)
    );
    if let Some(path) = lang.cmdline.iter().nth(0) {
        let mut cmd = Command::new(path);
        cmd.args(
            lang.cmdline
                .iter()
                .skip(1)
                .map(|a| if a == "{TIMEOUT}" { &timeout_arg } else { &a }),
        )
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
        debug!("spawning {:?}", cmd);

        let mut child = cmd.spawn().map_err(|e| format!("failed to exec: {}", e))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "stdin missing".to_owned())?;
        stdin
            .write_all(code.as_ref())
            .await
            .map_err(|e| format!("failed to write to stdin: {}", e))?;

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| format!("failed to wait for process: {}", e))?;
        let mut r = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
        if !output.status.success() {
            if !r.ends_with('\n') {
                r.push_str("\n");
            }
            if let Some(code) = output.status.code() {
                r.push_str(&format!("exited with status {}\n", code));
            } else if let Some(code) = output.status.signal() {
                r.push_str(&format!(
                    "signalled with {} ({})\n",
                    strsig(code),
                    strsigabbrev(code)
                ));
            } else {
                r.push_str("exited with unknown failure\n");
            }
        }
        Ok(r)
    } else {
        Err("empty cmdline".to_owned())
    }
}

pub async fn unix<'a, T, U>(
    lang: Arc<UnixSocketBackend>,
    timeout: Option<usize>,
    context: Option<U>,
    code: T,
) -> Result<String, String>
where
    T: AsRef<[u8]>,
    U: AsRef<[u8]>,
{
    let buf = make_persistent_input(timeout, context, code);

    let mut conn = UnixStream::connect(&lang.socket_addr)
        .await
        .map_err(|e| format!("error connecting: {}", e))?;
    conn.write_all(&buf)
        .await
        .map_err(|e| format!("error writing: {}", e))?;
    conn.flush()
        .await
        .map_err(|e| format!("error flushing: {}", e))?;

    let mut lenb = [0u8; 4];
    if let Some(timeout) = timeout {
        if let Ok(res) = time::timeout(
            Duration::from_secs(timeout as u64),
            conn.read_exact(&mut lenb),
        )
        .await
        {
            res
        } else {
            drop(do_persistent_timeout(&lang.timeout_cmdline).await);
            return Err("time limit exceeded".to_owned());
        }
    } else {
        conn.read_exact(&mut lenb).await
    }
    .map_err(|e| format!("error reading result length: {}", e))?;

    let outlen = Cursor::new(lenb).get_u32_le().min(1024) as usize;
    let mut buf = BytesMut::with_capacity(outlen);
    buf.resize(outlen, 0);
    conn.read_exact(&mut buf)
        .await
        .map_err(|e| format!("error reading result: {}", e))?;

    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn do_persistent_timeout(cmdline: &Option<Vec<String>>) -> Result<(), ()> {
    if let Some(cmdline) = cmdline.as_ref() {
        if let Some(path) = cmdline.iter().nth(0) {
            debug!("timeout kill: launching {:?}", cmdline);
            Command::new(path)
                .args(cmdline.iter().skip(1))
                .spawn()
                .map_err(|e| error!("failed to exec for timeout kill: {}", e))?
                .wait()
                .await
                .map_err(|e| error!("failed to exec for timeout kill: {}", e))?;
        }
    }

    Ok(())
}

fn make_persistent_input<T, U>(timeout: Option<usize>, context: Option<T>, code: U) -> BytesMut
where
    T: AsRef<[u8]>,
    U: AsRef<[u8]>,
{
    let timeout = timeout.unwrap_or(0usize) as u32;
    let contextb = context
        .as_ref()
        .map(|x| x.as_ref())
        .unwrap_or(&super::EMPTY_U8);
    let codeb = code.as_ref();
    let contextblen = contextb.len() as u32;
    let codeblen = codeb.len() as u32;

    let mut buf = BytesMut::with_capacity(12usize + contextblen as usize + codeblen as usize);
    buf.put_u32_le(timeout * 1000);
    buf.put_u32_le(contextblen);
    buf.put_u32_le(codeblen);
    buf.put(&contextb[..contextblen as usize]);
    buf.put(&codeb[..codeblen as usize]);
    buf
}
