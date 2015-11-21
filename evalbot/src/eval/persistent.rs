use std::sync::{mpsc, Arc, Mutex, Semaphore};
use byteorder::{ReadBytesExt, WriteBytesExt, NativeEndian};
use std::thread;
use std::io::{Read, Write};
use std::collections::VecDeque;
use std::process::{Child, ChildStdin, ChildStdout};
use std;
use eval::Evaluator;

#[derive(Clone)]
pub struct PersistentEvaluator {
    queue: Arc<Mutex<VecDeque<Request>>>,
    has_work: Arc<Semaphore>
}

enum Request {
    Terminate,
    Restart,
    Work { code: String, timeout: usize, reporter: mpsc::Sender<Output> }
}

struct Output {  
    success: bool,
    output: String
}

impl PersistentEvaluator {
    fn new() -> Self {
        PersistentEvaluator {
            queue: Arc::new(Mutex::new(VecDeque::new())),
            has_work: Arc::new(Semaphore::new(0))
        }
    }
}

impl Evaluator for PersistentEvaluator {
    #[cfg(not(unix))]
    fn eval(&self, _: &str, _: &str, _: usize) -> Result<String, String> {
        Err("not implemented".to_owned())
    }

    #[cfg(unix)]
    fn eval(&self, code: &str, _: &str, timeout: usize) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        let ref queue = self.queue;
        let ref has_work = self.has_work;

        let work = Request::Work { code: code.to_owned(), timeout: timeout, reporter: tx };
        queue.lock().unwrap().push_back(work);
        has_work.release();

        let result = match rx.recv() {
            Ok(x) => x,
            Err(x) => {
                println!("couldn't receive result: {:?}", x);
                Output { success: false, output: "something bad happened".to_owned() }
            }
        };

        if result.success {
            Ok(result.output)
        } else {
            Err(result.output)
        }
    }
}

fn worker_evaluate(stdin: &mut ChildStdin, stdout: &mut ChildStdout, work: &Request) -> Result<(), ()> {
    macro_rules! try_io {
        ($x:expr) => {
            match $x {
                Err(x) => { println!("couldn't communicate with child: {:?}", x); return Err(()); },
                Ok(x) => x
            }
        }
    };

    if let &Request::Work { ref code, timeout, ref reporter } = work {
        try_io!(stdin.write_i32::<NativeEndian>((timeout*1000) as i32));
        let bytes = code.as_bytes();
        try_io!(stdin.write_i32::<NativeEndian>(bytes.len() as i32));
        try_io!(stdin.write_all(bytes));
        try_io!(stdin.flush());

        let success = try_io!(stdout.read_u8()) == 1;
        let result_len = try_io!(stdout.read_i32::<NativeEndian>());
        let mut result_bytes = vec![0u8; result_len as usize];
        try_io!(stdout.read_exact(&mut result_bytes));

        // we'll just ignore this error, not going to restart child
        match reporter.send(Output { success: success, output: String::from_utf8_lossy(&result_bytes).into_owned() }) {
            Ok(_) => (),
            Err(x) => println!("couldn't return result: {:?}", x)
        };
        Ok(())
    } else {
        println!("worker_evaluate got something other than Request::Work");
        Ok(())
    }
}

fn worker<'a, F>(childfn: F, queue: Arc<Mutex<VecDeque<Request>>>, has_work: Arc<Semaphore>)
    where F : Fn() -> Child + Send + 'static {
    let mut terminate;
    loop {
        let mut evaluator = childfn();
        println!("started persistent child pid {}", evaluator.id());
        let mut stdin = evaluator.stdin.take().unwrap();
        let mut stdout = evaluator.stdout.take().unwrap();
        loop {
            has_work.acquire();
            let mut rvec = queue.lock().unwrap();
            let work = rvec.pop_front();
            std::mem::drop(rvec);

            if let Some(work) = work {
                match work {
                    Request::Terminate => {
                        println!("requested to terminate persistent child pid {}", evaluator.id());
                        terminate = true;
                        break;
                    },
                    Request::Restart => {
                        println!("requested to restart persistent child pid {}", evaluator.id());
                        terminate = false;
                        break;
                    },
                    Request::Work { .. } => {
                        let res = worker_evaluate(&mut stdin, &mut stdout, &work);
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
        println!("killing persistent child pid {}", evaluator.id());
        match evaluator.kill() { Err(x) => println!("failed to kill: {:?}", x), _ => () };
        if terminate { break; }
    }
}

pub fn new<F>(childfn: F) -> PersistentEvaluator
    where F : Fn() -> Child + Send + 'static {
    let ret = PersistentEvaluator::new();
    let (queue, has_work) = (ret.queue.clone(), ret.has_work.clone());
    thread::spawn(move || { worker(childfn, queue, has_work); });
    ret
}
