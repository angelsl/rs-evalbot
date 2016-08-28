use eval;
use std::ffi::OsStr;

#[derive(Clone, Debug)]
pub struct ExecLang {
    path: String,
    args: Vec<String>,
    timeout_arg: Option<String>
}

unsafe impl Send for ExecLang {}
unsafe impl Sync for ExecLang {}

impl ExecLang {
    pub fn new(path: String, args: Vec<String>, timeout_arg: Option<String>) -> Self {
        ExecLang { path: path, args: args, timeout_arg: timeout_arg }
    }

    fn exec_args(&self, timeout: Option<usize>) -> Vec<String> {
        if let (&Some(ref arg), Some(timeout)) = (&self.timeout_arg, timeout) {
            self.args
                .iter()
                .filter_map(|x| {
                    match x as &str {
                        "{TIMEOUT}" => Some(format!("{}{}", arg, timeout)),
                        _ => Some(x.to_owned()),
                    }
                })
                .collect::<Vec<String>>()
        } else {
            self.args.clone()
        }
    }
}

impl eval::Lang for ExecLang {
    fn eval(&self, code: &str, timeout: Option<usize>, _: Option<&str>) -> Result<String, String> {
        exec_wait(&self.path, &self.exec_args(timeout), code)
    }
}

fn exec_wait<S: AsRef<OsStr>>(command: &str, args: &[S], input: &str) -> Result<String, String> {
    use std::process::{Child, Command, Stdio};
    use std::io::Write;
    fn spawn<S: AsRef<OsStr>>(command: &str, args: &[S], merge_stderr: bool) -> Result<Child, String> {
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
