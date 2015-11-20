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
        Lang::Python => python::eval(&req.code, sandbox_path, timeout),
        _ => Err("invalid language".to_owned())
    }
}

pub fn eval_csharp(req: &Req, timeout: usize, eval: &csharp::Evaluator) -> Result<String, String> {
    match req.language {
        Lang::CSharp => csharp::eval(&req.code, timeout, eval),
        _ => Err("invalid language".to_owned())
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

pub mod csharp {
    use playpen;
    use std::sync::{mpsc, Arc, Mutex, Semaphore};
    use std::process::Child;
    use byteorder::{ReadBytesExt,WriteBytesExt,NativeEndian};
    use std::thread;
    use std::io::{Read, Write};
    use std::collections::VecDeque;
    use std;

    #[derive(Clone)]
    pub struct Evaluator {
        queue: Arc<Mutex<VecDeque<(String, usize, mpsc::Sender<(bool, String)>)>>>,
        has_work: Arc<Semaphore>
    }

    impl Evaluator {
        fn new() -> Self {
            Evaluator {
                queue: Arc::new(Mutex::new(VecDeque::new())),
                has_work: Arc::new(Semaphore::new(0))
            }
        }
    }

    fn worker<'a>(evaluator: Child, queue: Arc<Mutex<VecDeque<(String, usize, mpsc::Sender<(bool, String)>)>>>, has_work: Arc<Semaphore>) {
        macro_rules! try_io {
            ($x:expr) => {
                match $x {
                    Err(x) => panic!(x),
                    Ok(x) => x
                }
            }
        };
        let mut stdin = evaluator.stdin.unwrap();
        let mut stdout = evaluator.stdout.unwrap();
        loop {
            has_work.acquire();
            let mut rvec = queue.lock().unwrap();
            let work = rvec.pop_front();
            std::mem::drop(rvec);
            
            if let Some(work) = work {
                try_io!(stdin.write_i32::<NativeEndian>(work.1 as i32));
                let bytes = work.0.as_bytes();
                try_io!(stdin.write_i32::<NativeEndian>(bytes.len() as i32));
                try_io!(stdin.write_all(bytes));
                try_io!(stdin.flush());

                let success = try_io!(stdout.read_u8()) == 1;
                let result_len = try_io!(stdout.read_i32::<NativeEndian>());
                let mut result_bytes = vec![0u8; result_len as usize];
                try_io!(stdout.read_exact(&mut result_bytes));

                match work.2.send((success, String::from_utf8_lossy(&result_bytes).into_owned())) {
                    Ok(_) => (),
                    Err(x) => println!("couldn't return cseval result: {:?}", x)
                };
            }
        }
    }

    pub fn start_worker(sandbox: &str) -> Evaluator {
        let child = playpen::spawn(sandbox, "/usr/bin/mono", "mono_syscalls",
                                   &["/usr/local/bin/cseval.exe"],
                                   None,
                                   false);
        let ret = Evaluator::new();
        let (queue, has_work) = (ret.queue.clone(), ret.has_work.clone());
        thread::spawn(move || { worker(child.unwrap(), queue, has_work); });
        ret
    }

    #[cfg(not(unix))]
    pub fn eval(_: &str, _: usize) -> Result<String, String> {
        Ok("not implemented".to_owned())
    }

    #[cfg(unix)]
    pub fn eval(code: &str, timeout: usize, eval: &Evaluator) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        let queue = eval.queue.clone();
        let has_work = eval.has_work.clone();

        let work = (code.to_owned(), timeout, tx);
        queue.lock().unwrap().push_back(work);
        has_work.release();

        let result = match rx.recv() {
            Ok(x) => x,
            Err(x) => {
                println!("couldn't receive cseval result: {:?}", x);
                (false, "something bad happened".to_owned())
            }
        };
        
        if result.0 {
            Err(result.1)
        } else {
            Ok(result.1)
        }
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
