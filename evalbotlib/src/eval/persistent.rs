extern crate byteorder;
extern crate zombie;

use std;
use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use self::byteorder::{NativeEndian, ReadBytesExt, WriteBytesExt};

use util::ignore;
use {eval, playpen};

#[derive(Clone)]
pub struct ReplLang {
    cfg: ::LangCfg,
    process: Arc<Mutex<Child>>,
    stdinout: Arc<Mutex<(ChildStdin, ChildStdout)>>
}

impl std::fmt::Debug for ReplLang {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("ReplLang").field("cfg", &self.cfg).finish()
    }
}

unsafe impl Send for ReplLang {}
unsafe impl Sync for ReplLang {}

fn spawn_child(cfg: &::LangCfg) -> (Child, (ChildStdin, ChildStdout)) {
    let mut child = playpen::spawn(&cfg.binary_path,
                                   &cfg.args(false),
                                   false)
        .unwrap();
    let stdinout = (child.stdin.take().unwrap(), child.stdout.take().unwrap());
    (child, stdinout)
}

impl ReplLang {
    pub fn new(cfg: ::LangCfg) -> Self {
        let child = spawn_child(&cfg);
        ReplLang {
            cfg: cfg,
            process: Arc::new(Mutex::new(child.0)),
            stdinout: Arc::new(Mutex::new(child.1))
        }
    }

    fn eval_impl(&self, code: &str, with_timeout: bool, context_key: Option<&str>) -> Result<String, String> {
        let (txo, rx) = mpsc::channel();
        let stdinout = self.stdinout.clone();
        let code_bytes = code.as_bytes().to_owned();
        let key_bytes = context_key.unwrap_or("").as_bytes().to_owned();
        let timeout = if with_timeout { self.cfg.timeout.unwrap() } else { 0 };
        {
            // wait for response
            let tx = txo.clone();
            thread::spawn(move || {
                macro_rules! try_io {
                    ($x:expr) => {
                        match $x {
                            Ok(x) => x,
                            Err(_) => {
                                ignore(tx.send(Err("couldn't communicate with child".to_owned())));
                                return;
                            }
                        }
                    }
                };

                let (ref mut stdin, ref mut stdout) = *match stdinout.lock() {
                    Ok(x) => x,
                    Err(_) => {
                        ignore(tx.send(Err("could not lock stdinout".to_owned())));
                        return;
                    }
                };
                try_io!(stdin.write_i32::<NativeEndian>((timeout * 1000) as i32));
                try_io!(stdin.write_i32::<NativeEndian>(key_bytes.len() as i32));
                try_io!(stdin.write_i32::<NativeEndian>(code_bytes.len() as i32));
                try_io!(stdin.write_all(&key_bytes));
                try_io!(stdin.write_all(&code_bytes));
                try_io!(stdin.flush());

                let result_len = try_io!(stdout.read_i32::<NativeEndian>());
                if result_len > 1024 * 1024 {
                    ignore(tx.send(Err("response from child too large".to_owned())));
                    return;
                }
                let mut result_bytes = vec![0u8; result_len as usize];
                try_io!(stdout.read_exact(&mut result_bytes));
                ignore(tx.send(Ok(String::from_utf8_lossy(&result_bytes).into_owned())));
            });
        }

        if with_timeout {
            // timeout
            let timeout = (self.cfg.timeout.unwrap() as f64 * 1.5) as u64;
            thread::spawn(move || {
                thread::sleep(std::time::Duration::new(timeout, 0));
                ignore(txo.send(Err("timed out waiting for evaluator response".to_owned())));
            });
        }

        match rx.recv() {
            Ok(x) => x,
            Err(_) => Err("channel error communicating with communication thread".to_owned()),
        }
    }

    fn restart_process(&self, process: &mut Child, stdinout: &mut (ChildStdin, ChildStdout)) {
        kill_process(process);
        let spawn_result = spawn_child(&self.cfg);
        *process = spawn_result.0;
        *stdinout = spawn_result.1;
    }
}

impl eval::Lang for ReplLang {
    fn eval(&self, code: &str, with_timeout: bool, context_key: Option<&str>) -> Result<String, String> {
        if let Ok(mut process) = self.process.lock() {
            let result = self.eval_impl(code, with_timeout, context_key);
            if let Err(_) = result {
                if let Ok(mut stdinout) = self.stdinout.lock() {
                    self.restart_process(&mut *process, &mut *stdinout);
                }
            }
            result
        } else {
            Err("failed to lock process".to_owned())
        }
    }

    fn restart(&self) {
        if let (Ok(mut process), Ok(mut stdinout)) = (self.process.lock(), self.stdinout.lock()) {
            self.restart_process(&mut *process, &mut *stdinout);
        } else {
            println!("failed to lock mutexes for restart");
        }
    }

    fn is_persistent(&self) -> bool {
        true
    }
}

impl Drop for ReplLang {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.lock() {
            kill_process(&mut *process);
        }
    }
}

fn kill_process(process: &mut Child) {
    ignore(Command::new("sudo").args(&["kill", "-KILL"]).arg(format!("{}", process.id())).spawn());
    ignore(process.kill());
    zombie::collect_zombies();
}
