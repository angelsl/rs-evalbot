use std::fmt::Display;
use std::process::{Command, Stdio};

use tokio::prelude::*;
use tokio_process::CommandExt;
use tokio::{io::{read_exact, write_all}, net::unix::UnixStream};
use bytes::{BytesMut, IntoBuf, Buf, BufMut};

pub fn exec<'a, I, S, T>(
    path: &str,
    args: &I,
    timeout: Option<usize>,
    timeout_prefix: Option<&str>,
    code: T) -> impl Future<Item = String, Error = String> + 'a
        where
            for<'b> &'b I: IntoIterator<Item = &'b S>,
            S: AsRef<str> + PartialEq,
            T: AsRef<[u8]> + 'a {
    let timeout_arg = timeout
        .map(|t| format!("{}{}", timeout_prefix.unwrap_or(""), t));
    let timeout_arg_ref = timeout_arg.as_ref().map(String::as_str);
    let mut cmd = Command::new(path);
    cmd.args(args.into_iter()
            .filter_map(|a| if a.as_ref() == "{TIMEOUT}" {
                timeout_arg_ref
            } else {
                Some(a.as_ref())
            }))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    debug!("spawning {:?}", cmd);
    cmd.spawn_async()
        .map_err(|e| format!("failed to exec: {}", e))
        .into_future()
        .and_then(|mut child| {
            child.stdin().take().ok_or_else(|| "stdin missing".to_owned()).into_future()
                .and_then(|stdin|
                    write_all(stdin, code)
                        .map_err(|e| format!("failed to write to stdin: {}", e))
                )
                .and_then(|_| child.wait_with_output()
                    .map_err(|e| format!("failed to wait for process: {}", e)))
        })
        .map_err(|e| format!("unknown error in exec: {}", e))
        .map(|o| format!("{}{}",
            String::from_utf8_lossy(&o.stderr),
            String::from_utf8_lossy(&o.stdout),
        ))
}

pub fn unix<'a, T, U>(
    path: &str,
    timeout: Option<usize>,
    context: Option<U>,
    code: T) -> impl Future<Item = String, Error = String> + 'a
        where
            T: AsRef<[u8]>,
            U: AsRef<[u8]> {
    persistent(UnixStream::connect(path), make_persistent_input(timeout, context, code))
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
    buf.put_u32_le(timeout);
    buf.put_u32_le(contextblen);
    buf.put_u32_le(codeblen);
    buf.put(&contextb[..contextblen as usize]);
    buf.put(&codeb[..codeblen as usize]);
    buf
}

fn persistent<'a, F, G>(connfut: F, buf: BytesMut)
    -> impl Future<Item = String, Error = String> + 'a
        where
            F: Future<Item = G> + 'a,
            <F as Future>::Error: Display,
            G: AsyncRead + AsyncWrite + 'a {
    // FIXME potentially make this work without copying?
    // lifetime issues though
    connfut
        .map_err(|e| format!("error connecting: {}", e))
        .and_then(move |s| write_all(s, buf)
            .map_err(|e| format!("error writing: {}", e)))
        .and_then(|(s, _)| read_exact(s, BytesMut::with_capacity(4))
            .map_err(|e| format!("error reading result length: {}", e)))
        .and_then(|(s, lenb)|
            read_exact(s, BytesMut::with_capacity(lenb.into_buf().get_u32_le() as usize))
            .map_err(|e| format!("error reading result: {}", e)))
        .map(|(_, outb)| String::from_utf8_lossy(&outb).into_owned())
}
