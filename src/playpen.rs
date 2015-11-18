#[cfg(unix)]
pub fn exec(sandbox: &str, command: &str, args: &[&str], input: &str, timeout: usize) -> Result<(String, String), String> {
    use std::process::{Command,Stdio};
    use std::io::Write;
    let mut child = try!(Command::new("sudo")
        .arg("playpen")
        .arg(sandbox)
        .arg("--mount-proc")
        .arg("--user=rust")
        .arg(format!("--timeout={}", timeout))
        .arg("--syscalls-file=whitelist")
        .arg("--devices=/dev/urandom:r,/dev/null:w")
        .arg("--memory-limit=128")
        .arg("--")
        .arg(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn().map_err(|x| format!("couldn't playpen_exec; {}", x)));
    if let Some(ref mut x) = child.stdin { 
        try!(x.write_all(input.as_bytes())
             .map_err(|x| format!("couldn't write to stdin; {}", x)));
    } else {
        return Err("no stdin?".to_owned());
    }
    let output = try!(child.wait_with_output().
                      map_err(|x| format!("wait_with_output failed; {}", x)));
    Ok((String::from_utf8_lossy(&output.stdout).into_owned(), 
     String::from_utf8_lossy(&output.stderr).into_owned()))
}
