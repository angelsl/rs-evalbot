use std;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::process::{Command, ChildStdin, ChildStdout, ExitStatus};
use std::sync::{mpsc, Arc, Mutex, Semaphore};
use std::thread;
use std::time::Duration;

use byteorder::{ReadBytesExt, WriteBytesExt, NativeEndian};

use {cfg, playpen, eval};

#[derive(Clone)]
pub struct ReplLang {
    cfg: cfg::LangCfg,
    playpen_args: Vec<String>,
    sandbox_path: String,
    timeout: usize,
    queue: Arc<Mutex<VecDeque<Request>>>,
    has_work: Arc<Semaphore>
}

impl std::fmt::Debug for ReplLang {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "ReplLang {{ cfg: {:?} }}", self.cfg)
    }
}

unsafe impl Send for ReplLang {}
unsafe impl Sync for ReplLang {}

#[derive(Clone)]
enum Request {
    Terminate,
    Restart,
    Work {
        code: String,
        timeout: usize,
        reporter: mpsc::Sender<Output>
    }
}

#[derive(PartialEq)]
enum Status {
    EvalbotError,
    ChildError,
    Success
}

struct Output {
    success: Status,
    output: String
}

impl ReplLang {
    pub fn new(cfg: cfg::LangCfg,
               playpen_args: Vec<String>,
               sandbox_path: String,
               timeout: usize)
               -> Self {
        let ret = ReplLang {
            cfg: cfg,
            playpen_args: playpen_args,
            sandbox_path: sandbox_path,
            timeout: timeout,
            queue: Arc::new(Mutex::new(VecDeque::new())),
            has_work: Arc::new(Semaphore::new(0))
        };
        {
            let has_work = ret.has_work.clone();
            let queue = ret.queue.clone();
            let cfg = ret.cfg.clone();
            let sandbox_path = ret.sandbox_path.clone();
            let playpen_args = ret.playpen_args.clone();
            thread::spawn(move || {
                worker(has_work, queue, cfg, sandbox_path, playpen_args);
            });
        }
        ret
    }

    fn send_request(&self, req: Request) {
        self.queue.lock().unwrap().push_back(req);
        self.has_work.release();
    }
}

impl eval::Lang for ReplLang {
    #[cfg(not(unix))]
    fn eval(&self, _: &str) -> Result<String, String> {
        Err("not implemented".to_owned())
    }

    #[cfg(unix)]
    fn eval(&self, code: &str) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        self.send_request(Request::Work {
            code: code.to_owned(),
            timeout: self.timeout,
            reporter: tx
        });

        let result = match rx.recv() {
            Ok(x) => x,
            Err(x) => {
                println!("couldn't receive result: {:?}", x);
                Output {
                    success: Status::EvalbotError,
                    output: "something bad happened".to_owned()
                }
            }
        };

        // this basically controls whether the output is prefixed if it's in a channel
        if result.success == Status::Success { Ok(result.output) } else { Err(result.output) }
    }

    fn restart(&self) {
        self.send_request(Request::Restart);
    }

    fn terminate(&self) {
        self.send_request(Request::Terminate);
    }
}

fn worker(has_work: Arc<Semaphore>,
          queue: Arc<Mutex<VecDeque<Request>>>,
          cfg: cfg::LangCfg,
          sandbox_path: String,
          playpen_args: Vec<String>) {
    let mut terminate;
    loop {
        let mut evaluator = if let Ok(x) = playpen::spawn(&sandbox_path,
                                                          &cfg.binary_path,
                                                          &cfg.syscalls_path,
                                                          &playpen_args,
                                                          &cfg.binary_args,
                                                          None,
                                                          false) {
            x
        } else {
            thread::sleep(Duration::new(1, 0));
            continue;
        };
        println!("started persistent child pid {}", evaluator.id());
        let mut stdin = evaluator.stdin.take().unwrap();
        let stdout = Arc::new(Mutex::new(evaluator.stdout.take().unwrap()));
        loop {
            has_work.acquire();
            let mut rvec = queue.lock().unwrap();
            let work = rvec.pop_front();
            std::mem::drop(rvec);

            if let Some(work) = work {
                match work {
                    Request::Terminate => {
                        println!("requested to terminate persistent child pid {}",
                                 evaluator.id());
                        terminate = true;
                        break;
                    }

                    Request::Restart => {
                        println!("requested to restart persistent child pid {}",
                                 evaluator.id());
                        terminate = false;
                        break;
                    }
                    Request::Work { .. } => {
                        let res = worker_evaluate(&mut stdin, stdout.clone(), &work);
                        match res {
                            Ok(_) => (),
                            Err(_) => {
                                terminate = false;
                                break;
                            }
                        };
                    }
                }
            }
        }
        let pid = evaluator.id();
        println!("killing persistent child pid {}", pid);
        match sudo_kill(pid) {
            Err(x) => println!("failed to kill {}: {}", pid, x),
            Ok(x) => println!("kill result: {:?}", x),
        };
        // we only do this here because Child::kill does waitpid, to reap the process
        match evaluator.kill() {
            _ => (),
        };
        if terminate {
            break;
        }
    }
}

fn worker_evaluate(stdin: &mut ChildStdin,
                   stdout: Arc<Mutex<ChildStdout>>,
                   work: &Request)
                   -> Result<(), ()> {
    macro_rules! try_io {
        ($x:expr, $repr:expr) => {
            match $x {
                Err(x) => {
                    println!("couldn't communicate with child (1): {:?}", x);
                    match $repr.send(Output { success: Status::EvalbotError, output: "couldn't communicate with child (1)".to_owned() }) {
                        Ok(_) => (),
                        Err(x) => println!("couldn't report error (1): {:?}", x)
                    };
                    return Err(());
                },
                Ok(x) => x
            }
        }
    };

    macro_rules! try_io2 {
        ($x:expr, $repr:expr) => {
            match $x {
                Err(x) => {
                    println!("couldn't communicate with child (2): {:?}", x);
                    match $repr.send(Output { success: Status::EvalbotError, output: "couldn't communicate with child (2)".to_owned() }) {
                        Ok(_) => (),
                        Err(x) => println!("couldn't report error (2): {:?}", x)
                    };
                    return;
                },
                Ok(x) => x
            }
        }
    };

    if let Request::Work { ref code, timeout, ref reporter } = *work {
        try_io!(stdin.write_i32::<NativeEndian>((timeout * 1000) as i32),
                reporter);
        let bytes = code.as_bytes();
        try_io!(stdin.write_i32::<NativeEndian>(bytes.len() as i32),
                reporter);
        try_io!(stdin.write_all(bytes), reporter);
        try_io!(stdin.flush(), reporter);

        let (tx, rx) = mpsc::channel();

        {
            // wait for response
            let tx = tx.clone();
            thread::spawn(move || {
                let mut stdout = stdout.lock().unwrap();
                let success = try_io2!(stdout.read_u8(), tx) == 1;
                let result_len = try_io2!(stdout.read_i32::<NativeEndian>(), tx);
                if result_len > 1024 * 1024 {
                    match tx.send(Output {
                        success: Status::EvalbotError,
                        output: "response from child too large".to_owned()
                    }) {
                        _ => (),
                    };
                    return;
                }
                let mut result_bytes = vec![0u8; result_len as usize];
                try_io2!(stdout.read_exact(&mut result_bytes), tx);
                match tx.send(Output {
                    success: if success { Status::Success } else { Status::ChildError },
                    output: String::from_utf8_lossy(&result_bytes).into_owned()
                }) {
                    _ => (),
                };
            });
        }

        {
            // timeout
            let timeout = (timeout as f64 * 1.5) as u64;
            thread::spawn(move || {
                thread::sleep(::std::time::Duration::new(timeout, 0));
                match tx.send(Output {
                    success: Status::EvalbotError,
                    output: "timed out waiting for evaluator response".to_owned()
                }) {
                    _ => (),
                };
            });
        }
        let mut err = false;

        // this one receives from the two threads spawned above
        // Ok means we got a message through the channel
        // Err means both threads above died
        let result = match rx.recv() {
            Ok(x) => x,
            Err(_) => {
                err = true;
                Output {
                    success: Status::EvalbotError,
                    output: "couldn't receive result from communicator thread".to_owned()
                }
            }
        };

        if result.success == Status::EvalbotError {
            err = true;
        }

        // we'll just ignore this error, not going to restart child
        match reporter.send(result) {
            Ok(_) => (),
            Err(x) => println!("couldn't return result: {:?}", x),
        };

        // returning Err here will make the loop in worker(..) break
        // restarting the child
        if err { Err(()) } else { Ok(()) }
    } else {
        println!("worker_evaluate got something other than Request::Work");
        Ok(())
    }
}

fn sudo_kill(pid: u32) -> Result<ExitStatus, String> {
    try!(Command::new("sudo")
             .args(&["kill", "-KILL"])
             .arg(format!("{}", pid))
             .spawn()
             .map_err(|x| format!("couldn't spawn sudo kill: {:?}", x)))
        .wait()
        .map_err(|x| format!("couldn't SIGKILL: {:?}", x))
}
