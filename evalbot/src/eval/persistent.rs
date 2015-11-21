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

fn worker_evaluate(stdin: &mut ChildStdin, stdout: Arc<Mutex<ChildStdout>>, work: &Request) -> Result<(), ()> {
    macro_rules! try_io {
        ($x:expr, $repr:expr) => {
            match $x {
                Err(x) => {
                    println!("couldn't communicate with child (1): {:?}", x);
                    match $repr.send(Output { success: false, output: "couldn't communicate with child (1)".to_owned() }) {
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
                    match $repr.send(Err(Output { success: false, output: "couldn't communicate with child (2)".to_owned() })) {
                        Ok(_) => (),
                        Err(x) => println!("couldn't report error (2): {:?}", x)
                    };
                    return;
                },
                Ok(x) => x
            }
        }
    };    

    if let &Request::Work { ref code, timeout, ref reporter } = work {
        println!("got work, sending");
        try_io!(stdin.write_i32::<NativeEndian>((timeout*1000) as i32), reporter);
        let bytes = code.as_bytes();
        try_io!(stdin.write_i32::<NativeEndian>(bytes.len() as i32), reporter);
        try_io!(stdin.write_all(bytes), reporter);
        try_io!(stdin.flush(), reporter);
        println!("sent work");
        
        let (tx, rx) = mpsc::channel();

        { // wait for response
            let tx = tx.clone();
            thread::spawn(move || {
                let mut stdout = stdout.lock().unwrap();
                let success = try_io2!(stdout.read_u8(), tx) == 1;
                let result_len = try_io2!(stdout.read_i32::<NativeEndian>(), tx);
                let mut result_bytes = vec![0u8; result_len as usize];
                try_io2!(stdout.read_exact(&mut result_bytes), tx);
                match tx.send(Ok(Output { success: success, output: String::from_utf8_lossy(&result_bytes).into_owned() })) { _ => () };
            });
        }

        { // timeout
            let timeout = timeout;
            thread::spawn(move || {
                thread::sleep(::std::time::Duration::new(timeout as u64, 0));
                match tx.send(Err(Output { success: false, output: "timed out waiting for evaluator response".to_owned() })) { _ => () };
            });
        }
        let mut err = false;
        let result = match rx.recv() {
            Ok(x) => x,
            Err(_) => { err = true; Ok(Output { success: false, output: "couldn't receive result from communicator thread".to_owned() }) }
        };
        let result = match result {
            Ok(x) => x,
            Err(x) => { err = true; x }
        };
        println!("got response");
        // we'll just ignore this error, not going to restart child
        match reporter.send(result) {
            Ok(_) => (),
            Err(x) => println!("couldn't return result: {:?}", x)
        };
        if err { Err(()) } else { Ok(()) }
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
        let stdout = Arc::new(Mutex::new(evaluator.stdout.take().unwrap()));
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
