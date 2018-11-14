use std::process::{Command, Stdio};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use std::os::unix::process::ExitStatusExt;

use tokio::prelude::*;
use tokio::prelude::future::Either;
use tokio::timer::timeout;
use tokio_process::CommandExt;
use tokio::{io::{flush, read_exact, write_all}, net::unix::UnixStream};
use bytes::{BytesMut, Buf, BufMut};

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
        _ => "Unknown signal"
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
        _ => "(unknown)"
    }
}

pub fn exec<'a, T>(
    lang: Arc<ExecBackend>,
    timeout: Option<usize>,
    code: T) -> impl Future<Item = String, Error = String> + 'a
        where T: AsRef<[u8]> + 'a {
    let timeout_arg = timeout
        .map(|t| format!("{}{}", lang.timeout_prefix.as_ref().map(String::as_str).unwrap_or(""), t));
    let timeout_arg_ref = timeout_arg.as_ref().map(String::as_str);
    if let Some(path) = lang.cmdline.iter().nth(0) {
        let mut cmd = Command::new(path);
        cmd.args(lang.cmdline.iter()
            .skip(1)
            .filter_map(|a| if a == "{TIMEOUT}" {
                timeout_arg_ref
            } else {
                Some(a.as_ref())
            }))
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        debug!("spawning {:?}", cmd);
        Either::A(cmd.spawn_async()
            .map_err(|e| format!("failed to exec: {}", e))
            .into_future()
            .and_then(|mut child| {
                child.stdin().take().ok_or_else(|| "stdin missing".to_owned()).into_future()
                    .and_then(|stdin|
                        write_all(stdin, code)
                            .map_err(|e| format!("failed to write to stdin: {}", e))
                    )
                    .and_then(move |_| {
                        let stdout = match child.stdout().take() {
                            // FIXME configurable max
                            Some(io) => Either::A(helper::read_up_to(io, vec![0u8; 1024])
                                .map(|(_, mut v, s)| {
                                    v.truncate(s);
                                    v
                                })),
                            None => Either::B(Ok(Vec::new()).into_future()),
                        };
                        let stderr = match child.stderr().take() {
                            // FIXME configurable max
                            Some(io) => Either::A(helper::read_up_to(io, vec![0u8; 1024])
                                .map(|(_, mut v, s)| {
                                    v.truncate(s);
                                    v
                                })),
                            None => Either::B(Ok(Vec::new()).into_future()),
                        };
                        child.join3(stdout, stderr)
                    }.map_err(|e| format!("failed to wait for process: {}", e)))
            })
            .map_err(|e| format!("unknown error in exec: {}", e))
            .map(|(status, stdout, stderr)| {
                let mut r = format!("{}{}",
                    String::from_utf8_lossy(&stderr),
                    String::from_utf8_lossy(&stdout),
                );
                if !status.success() {
                    if !r.ends_with('\n') {
                        r.push_str("\n");
                    }
                    if let Some(code) = status.code() {
                        r.push_str(&format!("exited with status {}\n", code));
                    } else if let Some(code) = status.signal() {
                        r.push_str(&format!("signalled with {} ({})\n", strsig(code), strsigabbrev(code)));
                    } else {
                        r.push_str("exited with unknown failure\n");
                    }
                }
                r
            }))
    } else {
        Either::B(Err("empty cmdline".to_owned()).into_future())
    }
}

macro_rules! persistent {
    ($lang:expr, $connfut:expr, $timeout:expr, $buf:expr) => ({
        let buf = $buf;
        let fut = $connfut
            .map_err(|e| format!("error connecting: {}", e))
            .and_then(move |s| write_all(s, buf)
                .map_err(|e| format!("error writing: {}", e)))
            .and_then(|(s, _)| flush(s)
                .map_err(|e| format!("error flushing: {}", e)))
            .and_then(|s| read_exact(s, [0u8; 4])
                .map_err(|e| format!("error reading result length: {}", e)));
        if let Some(timeout) = $timeout {
            Either::A(fut.timeout(Duration::from_secs(timeout as u64)))
        } else {
            Either::B(fut.map_err(|e| timeout::Error::inner(e)))
        }.then(move |r| match r {
            Ok((s, lenb)) => Either::A(read_exact(s, {
                // FIXME configurable max
                let outlen = Cursor::new(lenb).get_u32_le().min(1024) as usize;
                let mut buf = BytesMut::with_capacity(outlen);
                buf.resize(outlen, 0);
                buf
            }).map_err(|e| format!("error reading result: {}", e))
                .map(|(_, ref outb)| String::from_utf8_lossy(outb).into_owned())),
            Err(e) => Either::B(if e.is_elapsed() {
                do_persistent_timeout(&$lang.timeout_cmdline);
                Ok("time limit exceeded".to_owned()).into_future()
            } else {
                Err(format!("error from timeout: {}", e)).into_future()
            })
        })
    });
}

pub fn unix<'a, T, U>(
    lang: Arc<UnixSocketBackend>,
    timeout: Option<usize>,
    context: Option<U>,
    code: T) -> impl Future<Item = String, Error = String> + 'a
        where
            T: AsRef<[u8]>,
            U: AsRef<[u8]> {
    persistent!(lang,
        UnixStream::connect(&lang.socket_addr),
        timeout,
        make_persistent_input(timeout, context, code))
}

fn do_persistent_timeout(cmdline: &Option<Vec<String>>) {
    if let Some(cmdline) = cmdline.as_ref() {
        if let Some(path) = cmdline.iter().nth(0) {
            debug!("timeout kill: launching {:?}", cmdline);
            tokio::spawn(Command::new(path)
                .args(cmdline.iter().skip(1))
                .spawn_async()
                .map_err(|e| error!("failed to exec for timeout kill: {}", e))
                .into_future()
                .and_then(|c| c
                    .map_err(|e| error!("failed to exec for timeout kill: {}", e)))
                    .map(|_| ()));
        }
    }
}

fn make_persistent_input<T, U>(timeout: Option<usize>, context: Option<T>, code: U) -> BytesMut
    where
        T: AsRef<[u8]>,
        U: AsRef<[u8]> {
    let timeout = timeout.unwrap_or(0usize) as u32;
    let contextb = context.as_ref().map(|x| x.as_ref()).unwrap_or(&super::EMPTY_U8);
    let codeb = code.as_ref();
    let contextblen = contextb.len() as u32;
    let codeblen = codeb.len() as u32;

    let mut buf = BytesMut::with_capacity(12usize + contextblen as usize + codeblen as usize);
    buf.put_u32_le(timeout*1000);
    buf.put_u32_le(contextblen);
    buf.put_u32_le(codeblen);
    buf.put(&contextb[..contextblen as usize]);
    buf.put(&codeb[..codeblen as usize]);
    buf
}

mod helper {
    // https://docs.rs/tokio-io/0.1.10/src/tokio_io/io/read_exact.rs.html

    use std::io;
    use std::mem;

    use futures::{Poll, Future};

    use tokio::io::AsyncRead;

    #[derive(Debug)]
    pub struct ReadUpTo<A, T> {
        state: State<A, T>,
    }

    #[derive(Debug)]
    enum State<A, T> {
        Reading {
            a: A,
            buf: T,
            pos: usize,
        },
        Empty,
    }

    pub fn read_up_to<A, T>(a: A, buf: T) -> ReadUpTo<A, T>
        where A: AsyncRead,
            T: AsMut<[u8]>,
    {
        ReadUpTo {
            state: State::Reading {
                a,
                buf,
                pos: 0,
            },
        }
    }

    impl<A, T> Future for ReadUpTo<A, T>
        where A: AsyncRead,
            T: AsMut<[u8]>,
    {
        type Item = (A, T, usize);
        type Error = io::Error;

        fn poll(&mut self) -> Poll<(A, T, usize), io::Error> {
            match self.state {
                State::Reading { ref mut a, ref mut buf, ref mut pos } => {
                    let buf = buf.as_mut();
                    while *pos < buf.len() {
                        let n = try_ready!(a.poll_read(&mut buf[*pos..]));
                        *pos += n;
                        if n == 0 {
                            break;
                        }
                    }
                }
                State::Empty => panic!("poll a ReadUpTo after it's done"),
            }

            match mem::replace(&mut self.state, State::Empty) {
                State::Reading { a, buf, pos } => Ok((a, buf, pos).into()),
                State::Empty => panic!(),
            }
        }
    }
}
