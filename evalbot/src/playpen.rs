use std::process::{Command, Stdio, Child};
use std::io::Write;

pub fn spawn(sandbox: &str,
             command: &str,
             syscalls: &str,
             args: &[&str],
             timeout: Option<usize>,
             merge_stderr: bool)
             -> Result<Child, String> {
    let mut cmd = Command::new("sudo");
    cmd.arg("playpen")
       .arg(sandbox)
       .arg("--mount-proc")
       .arg("--user=eval");
    if let Some(x) = timeout {
        cmd.arg(format!("--timeout={}", x));
    }
    cmd.arg(format!("--syscalls-file={}", syscalls))
       .arg("--devices=/dev/urandom:r,/dev/null:w")
       .arg("--memory-limit=128")
       .arg("--")
       .arg(command)
       .args(args)
       .stdin(Stdio::piped())
       .stdout(Stdio::piped());
    if merge_stderr {
        cmd.stderr(Stdio::piped());
    } else {
        cmd.stderr(Stdio::inherit());
    }
    cmd.spawn().map_err(|x| format!("couldn't playpen_exec; {}", x))
}

pub fn exec_wait(sandbox: &str,
                 command: &str,
                 syscalls: &str,
                 args: &[&str],
                 input: &str,
                 timeout: usize)
                 -> Result<String, String> {
    let mut child = try!(spawn(sandbox, command, syscalls, args, Some(timeout), true));
    if let Some(ref mut x) = child.stdin {
        try!(x.write_all(input.as_bytes())
              .map_err(|x| format!("couldn't write to stdin; {}", x)));
    } else {
        return Err("no stdin?".to_owned());
    }
    let output = try!(child.wait_with_output()
                           .map_err(|x| format!("wait_with_output failed; {}", x)));
    Ok({
        let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
        if !out.ends_with("\n") { out.push_str("\n"); }
        out.push_str(&*String::from_utf8_lossy(&output.stderr));
        out})
}
