use std::process::{Child, Command, Stdio};
use std::io::Write;
use std::ffi::OsStr;

pub fn spawn<S: AsRef<OsStr>>(
    command: &str,
    args: &[S],
    merge_stderr: bool)
                              -> Result<Child, String> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped());
    if merge_stderr {
        cmd.stderr(Stdio::piped());
    } else {
        cmd.stderr(Stdio::inherit());
    }
    cmd.spawn().map_err(|x| format!("couldn't playpen_exec; {}", x))
}

pub fn exec_wait<S: AsRef<OsStr>>(
    command: &str,
    args: &[S],
    input: &str)
                                  -> Result<String, String> {
    let mut child = try!(spawn(command, args, true));
    if let Some(ref mut x) = child.stdin {
        try!(x.write_all(input.as_bytes()).map_err(|x| format!("couldn't write to stdin; {}", x)));
    } else {
        return Err("no stdin?".to_owned());
    }
    let output = try!(child.wait_with_output().map_err(|x| format!("wait_with_output failed; {}", x)));
    Ok({
        let mut out = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        out.push_str("\n");
        out.push_str(String::from_utf8_lossy(&output.stderr).trim());
        out.trim().to_owned()
    })
}
