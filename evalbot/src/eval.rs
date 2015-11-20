#[derive(Debug)]
pub struct Req {
    pub is_channel: bool,
    pub sender: String,
    pub target: String,
    pub code: String,
    pub language: Lang
}

#[derive(Debug)]
pub enum Lang {
    Rust,
    RustRaw,
    CSharp,
    Python
}

impl ::std::str::FromStr for Lang {
    type Err = ();

    fn from_str(s: &str) -> Result<Lang, ()> {
        match &s.to_lowercase() as &str {
            "rust" | "rs" => Ok(Lang::Rust),
            "rust!" | "rs!" => Ok(Lang::RustRaw),
            "csharp" | "cs" => Ok(Lang::CSharp),
            "python" | "py" => Ok(Lang::Python),
            _ => Err(())
        }
    }
}

pub fn eval(req: &Req, sandbox_path: &str, timeout: usize) -> Result<String, String> {
    match req.language {
        Lang::Rust => rust::eval(&req.code, sandbox_path, timeout, false),
        Lang::RustRaw => rust::eval(&req.code, sandbox_path, timeout, true),
        Lang::CSharp => csharp::eval(&req.code, sandbox_path, timeout),
        Lang::Python => python::eval(&req.code, sandbox_path, timeout),
    }
}

mod rust {
    use playpen;

    fn expr_to_program(expr: &str) -> String {
        format!(
r#"#![allow(dead_code, unused_variables)]
fn show<T: std::fmt::Debug>(e: T) {{ println!("{{:?}}", e) }}
fn main() {{
    show({{
        {}
    }});
}}"#, expr)
    }

    #[cfg(not(unix))]
    pub fn eval(code: &str, _: &str, _: usize, _: bool) -> Result<String, String> {
        if let Ok(x) = code.parse::<usize>() {
            Ok(std::iter::repeat("X").take(x).collect::<String>())
        } else {
            Err("not a number".to_owned())
        }
    }

    #[cfg(unix)]
    pub fn eval(code: &str, sandbox: &str, timeout: usize, raw: bool) -> Result<String, String> {
        use std::borrow::Cow;
        let rust_eval_script =
r#"set -o errexit
rustc - -o ./out "$@"
exec ./out"#;

        let code = if raw { Cow::Borrowed(code) } else { Cow::Owned(expr_to_program(code)) };

        playpen::exec_wait(sandbox, "/usr/bin/dash", "rust_syscalls",
                           &["-c", rust_eval_script, "evaluate", "-C","opt-level=2"],
                           &*code,
                           timeout)
    }
}

mod csharp {
    use playpen;

    #[cfg(not(unix))]
    pub fn eval(_: &str, _: &str, _: usize) -> Result<String, String> {
        Ok("not implemented".to_owned())
    }

    #[cfg(unix)]
    pub fn eval(code: &str, sandbox: &str, timeout: usize) -> Result<String, String> {
        playpen::exec_wait(sandbox, "/usr/bin/mono", "mono_syscalls",
                           &["/usr/lib/mono/4.5/csharp.exe"],
                           &format!("{}\nquit\n", code),
                           timeout)
    }
}

mod python {
    use playpen;

    #[cfg(not(unix))]
    pub fn eval(_: &str, _: &str, _: usize) -> Result<String, String> {
        Ok("not implemented".to_owned())
    }

    #[cfg(unix)]
    pub fn eval(code: &str, sandbox: &str, timeout: usize) -> Result<String, String> {
        playpen::exec_wait(sandbox, "/usr/bin/python", "python_syscalls",
                           &["-ic", "import sys;sys.ps1='';sys.ps2=''"],
                           &format!("{}\nquit()\n", code.trim()),
                           timeout)
    }
}
