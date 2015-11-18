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
    CSharp
}

impl ::std::str::FromStr for Lang {
    type Err = ();

    fn from_str(s: &str) -> Result<Lang, ()> {
        match &s.to_lowercase() as &str {
            "rust" => Ok(Lang::Rust),
            "rustraw" => Ok(Lang::RustRaw),
            "cs" => Ok(Lang::CSharp),
            "csharp" => Ok(Lang::CSharp),
            _ => Err(())
        }
    }
}

pub fn eval(req: &Req, sandbox_path: &str, timeout: usize) -> String {
    match req.language {
        Lang::Rust => rust::eval(&req.code, sandbox_path, timeout, false),
        Lang::RustRaw => rust::eval(&req.code, sandbox_path, timeout, true),
        Lang::CSharp => "not implemented".to_owned()
    }
}

mod rust {
    use ::playpen;

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
    pub fn eval(code: &str, _: &str, _: usize, _: bool) -> String {
        if let Ok(x) = code.parse::<usize>() {
            std::iter::repeat("X").take(x).collect::<String>()
        } else {
            "not a number".to_owned()
        }
    }

#[cfg(unix)]
    pub fn eval(code: &str, sandbox: &str, timeout: usize, raw: bool) -> String {
        use ::std::borrow::Cow;
        let rust_eval_script =
r#"set -o errexit
rustc - -o ./out "$@"
printf '\377' # 255 in octal
exec ./out"#;
        
        let code = if raw { Cow::Borrowed(code) } else { Cow::Owned(expr_to_program(code)) };

        let (stdout, stderr) = match playpen::exec_wait(sandbox, "/usr/bin/dash", "rust_syscalls",
                                                        &["-c", rust_eval_script, "evaluate", "-C","opt-level=2"],
                                                        &*code,
                                                        timeout) {
            Ok(x) => x,
            Err(x) => return x
        };
        let stdout = stdout.replace("\u{FFFD}", "");
        let mut out = String::new();
        for line in stdout.lines() {
            out.push_str(&format!("stdout: {}\n", line));
        }
        for line in stderr.lines() {
            out.push_str(&format!("stderr: {}\n", line));
        }
        out
    }
}
