use std;

#[derive(Debug)]
pub struct Req {
    pub is_channel: bool,
    pub sender: String,
    pub target: String,
    pub code: String,
    pub language: Lang
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    Rust,
    RustRaw,
    CSharp,
    Python
}

impl std::str::FromStr for Lang {
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

pub fn evaluator(lang: Lang, sandbox: &str) -> Box<Evaluator> {
    match lang {
        Lang::Rust => Box::new(rust::RustEvaluator { raw: false }),
        Lang::RustRaw => Box::new(rust::RustEvaluator { raw: true }),
        Lang::Python => Box::new(python::evaluator(sandbox)),
        Lang::CSharp => Box::new(csharp::evaluator(sandbox))
    }
}

pub trait Evaluator: Send + Sync + 'static {
    fn eval(&self, code: &str, sandbox: &str, timeout: usize) -> Result<String, String>;
}

mod persistent;
mod csharp {
    use playpen;
    use std::process::Child;
    use eval::persistent;

    fn spawn_child(sandbox: &str) -> Child {
        playpen::spawn(sandbox, "/usr/bin/mono", "mono_syscalls",
                       &["/usr/local/bin/cseval.exe"],
                       None,
                       false).unwrap()
    }

    pub fn evaluator(sandbox: &str) -> persistent::PersistentEvaluator {
        let sandbox = sandbox.to_owned();
        persistent::new(move || { spawn_child(&sandbox) })
    }
}

mod rust {
    use playpen;
    use eval::Evaluator;

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

    pub struct RustEvaluator { pub raw: bool }

    impl Evaluator for RustEvaluator {
        #[cfg(not(unix))]
        fn eval(&self, code: &str, _: &str, _: usize) -> Result<String, String> {
            if let Ok(x) = code.parse::<usize>() {
                Ok(std::iter::repeat("X").take(x).collect::<String>())
            } else {
                Err("not a number".to_owned())
            }
        }

        #[cfg(unix)]
        fn eval(&self, code: &str, sandbox: &str, timeout: usize) -> Result<String, String> {
            use std::borrow::Cow;
            let rust_eval_script =
r#"set -o errexit
rustc - -o ./out "$@"
exec ./out"#;

            let code = if self.raw { Cow::Borrowed(code) } else { Cow::Owned(expr_to_program(code)) };

            playpen::exec_wait(sandbox, "/usr/bin/dash", "rust_syscalls",
                               &["-c", rust_eval_script, "evaluate", "-C","opt-level=2"],
                               &*code,
                               timeout)
        }
    }
}

mod python {
    use playpen;
    use std::process::Child;
    use eval::persistent;

    fn spawn_child(sandbox: &str) -> Child {
        playpen::spawn(sandbox, "/usr/bin/python", "python_syscalls",
                       &["/usr/local/bin/pyeval.py"],
                       None,
                       false).unwrap()
    }

    pub fn evaluator(sandbox: &str) -> persistent::PersistentEvaluator {
        let sandbox = sandbox.to_owned();
        persistent::new(move || { spawn_child(&sandbox) })
    }
}
