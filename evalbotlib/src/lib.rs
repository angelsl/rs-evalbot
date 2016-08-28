#![feature(plugin, pattern, fnbox, question_mark, stmt_expr_attributes)]
#![plugin(clippy)]

extern crate crossbeam;
extern crate rustc_serialize;

use crossbeam::sync::MsQueue;
use std::boxed::FnBox;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;

mod eval;
pub mod util;

pub type CallbackFnBox = Box<FnBox(Response) + Send>;

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
pub struct EvalSvcCfg {
    pub timeout: usize,
    pub eval_threads: usize,
    pub languages: Vec<LangCfg>
}

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
pub struct LangCfg {
    pub timeout: Option<usize>,
    pub timeout_opt: Option<String>,
    pub name: String,
    pub code_before: Option<String>,
    pub code_after: Option<String>,
    pub binary_path: Option<String>,
    pub binary_args: Option<Vec<String>>,
    pub binary_timeout_arg: Option<String>,
    pub network_address: Option<String>,
    pub socket_address: Option<String>
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
    languages: Arc<HashMap<String, (Arc<eval::Lang>, LangCfg)>>,
    threads: usize
}

enum Message {
    Request(Arc<eval::Lang>, String, CallbackFnBox, Option<usize>, Option<String>),
    Terminate
}

impl EvalSvc {
    pub fn new(cfg: EvalSvcCfg) -> Self {
        let timeout = cfg.timeout;
        let langs = cfg.languages
            .into_iter()
            .map(|cfg| LangCfg { timeout: Some(cfg.timeout.unwrap_or(timeout)), ..cfg })
            .map(|cfg| (cfg.name.clone(), (eval::new(&cfg), cfg)))
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
        if let Some(&(ref lang, ref cfg)) = self.languages.get(lang) {
            self.send_message(Message::Request(lang.clone(),
                                               wrap_code(&code, cfg),
                                               callback,
                                               if with_timeout {
                                                   Some(cfg.timeout.unwrap())
                                               } else {
                                                   None
                                               },
                                               context_key));
        } else {
            callback(Response::NoSuchLanguage);
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

fn wrap_code(raw: &str, cfg: &LangCfg) -> String {
    let mut code = String::with_capacity(raw.len());

    if let Some(ref prefix) = cfg.code_before {
        code.push_str(prefix);
    }

    code.push_str(raw);

    if let Some(ref postfix) = cfg.code_after {
        code.push_str(postfix);
    }

    code
}
