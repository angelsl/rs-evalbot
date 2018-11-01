use std::process::{Command, Stdio};
use std::ffi::{OsStr, OsString};

use futures::Future;
use futures::future::IntoFuture;
use tokio_process::{CommandExt, Child};
use tokio::io::write_all;
use std::io::Write;

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
