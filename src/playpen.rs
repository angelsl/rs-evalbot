use std::process::{Command,Stdio,Child};
use std::io::Write;

pub fn spawn(sandbox: &str, command: &str, syscalls: &str, args: &[&str], timeout: usize) -> Result<Child, String> {
    use std::os::unix::io::{FromRawFd,RawFd};
    Command::new("sudo")
        .arg("playpen")
        .arg(sandbox)
        .arg("--mount-proc")
        .arg("--user=rust")
        .arg(format!("--timeout={}", timeout))
        .arg(format!("--syscalls-file={}", syscalls))
        .arg("--devices=/dev/urandom:r,/dev/null:w")
        .arg("--memory-limit=128")
        .arg("--")
        .arg(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(unsafe { Stdio::from_raw_fd(1 as RawFd) })
        .spawn().map_err(|x| format!("couldn't playpen_exec; {}", x))
}

pub fn exec_wait(sandbox: &str, command: &str, syscalls: &str, args: &[&str], input: &str, timeout: usize) -> Result<String, String> {
    let mut child = try!(spawn(sandbox, command, syscalls, args, timeout));
    if let Some(ref mut x) = child.stdin { 
        try!(x.write_all(input.as_bytes())
             .map_err(|x| format!("couldn't write to stdin; {}", x)));
    } else {
        return Err("no stdin?".to_owned());
    }
    let output = try!(child.wait_with_output().
                      map_err(|x| format!("wait_with_output failed; {}", x)));
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
