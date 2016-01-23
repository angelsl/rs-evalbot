#![feature(plugin, pattern, fnbox)]
#![plugin(clippy)]

extern crate crossbeam;
extern crate rustc_serialize;

use std::sync::Arc;
use std::collections::HashMap;
use std::boxed::FnBox;
use std::thread;

use crossbeam::sync::MsQueue;

mod eval;
mod playpen;
mod worker;
pub mod util;

pub type CallbackFnBox = Box<FnBox(Response) + Send>;

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
pub struct EvalSvcCfg {
    pub sandbox_dir: String,
    pub playpen_timeout: usize,
    pub playpen_args: Vec<String>,
    pub eval_threads: usize,
    pub languages: Vec<LangCfg>
}

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
pub struct LangCfg {
    pub syscalls_path: String,
    pub binary_path: String,
    pub binary_args: Vec<String>,
    pub persistent: bool,
    pub name: String,
    pub code_before: Option<String>,
    pub code_after: Option<String>
}

unsafe impl Send for EvalSvcCfg {}
unsafe impl Send for LangCfg {}

pub enum Response {
    NoSuchLanguage,
    Error(String),
    Success(String)
}

#[derive(Clone)]
pub struct EvalSvc {
    queue: Arc<MsQueue<Message>>,
    languages: Arc<HashMap<String, Arc<eval::Lang>>>,
    threads: usize
}

enum Message {
    Request(Arc<eval::Lang>, String, CallbackFnBox, bool, Option<String>),
    Terminate
}

impl EvalSvc {
    pub fn new(cfg: EvalSvcCfg) -> Self {
        let langs = cfg.languages
                       .iter()
                       .map(|x| {
                           (x.name.clone(),
                            eval::new(x.clone(),
                                      cfg.playpen_args.clone(),
                                      cfg.sandbox_dir.clone(),
                                      cfg.playpen_timeout.clone()))
                       })
                       .collect::<HashMap<_, _>>();
        let ret = EvalSvc {
            queue: Arc::new(MsQueue::new()),
            threads: cfg.eval_threads,
            languages: Arc::new(langs)
        };
        for _ in 0..ret.threads {
            ret.spawn_thread();
        }
        ret
    }

    pub fn eval(&self,
        lang: &str,
        code: String,
        with_timeout: bool,
        context_key: Option<String>,
        callback: CallbackFnBox) {
        if let Some(lang) = self.languages.get(lang) {
            self.send_message(Message::Request(lang.clone(), code, callback, with_timeout, context_key));
        } else {
            callback(Response::NoSuchLanguage);
        }
    }

    pub fn restart(&self, lang: &str) -> Result<(), ()> {
        if let Some(lang) = self.languages.get(lang) {
            lang.restart();
            Ok(())
        } else {
            Err(())
        }
    }

    fn send_message(&self, msg: Message) {
        self.queue.push(msg);
    }

    fn spawn_thread(&self) {
        let queue = self.queue.clone();
        thread::spawn(move || {
            worker(queue);
        });
    }
}

impl Drop for EvalSvc {
    fn drop(&mut self) {
        for _ in 0..self.threads {
            self.queue.push(Message::Terminate);
        }
    }
}

fn worker(queue: Arc<MsQueue<Message>>) {
    loop {
        let msg = queue.pop();
        match msg {
            Message::Terminate => break,
            Message::Request(lang, code, callback, with_timeout, context_key) => {
                callback(match lang.eval(&code, with_timeout, context_key.as_ref().map(|x| x as &str)) {
                    Ok(x) => Response::Success(x),
                    Err(x) => Response::Error(x),
                });
            }
        };
    }
}
