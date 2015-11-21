#![feature(semaphore)]
#![feature(read_exact)]

extern crate irc;
extern crate rustc_serialize;
extern crate toml;
extern crate byteorder;

mod playpen;
mod cfg;
mod util;
mod eval;

use irc::client::prelude::*;
use std::sync::{Arc, Mutex, Semaphore};
use std::thread;
use std::collections::{VecDeque, HashMap};

use cfg::EvalbotCfg;
use eval::Req;

macro_rules! send_msg {
    ($conn:expr, $dest:expr, $line:expr) => {
        match $conn.send_privmsg($dest, $line) {
            Err(x) => println!("failed to send message: {}", x),
            _ => ()
        }
    };
}

#[derive(Clone)]
struct State {
    requests: Arc<Mutex<VecDeque<Req>>>,
    has_work: Arc<Semaphore>,
    cfg: Arc<EvalbotCfg>,
    evaluators: Arc<HashMap<eval::Lang, Box<eval::Evaluator>>>
}

fn evaluate_loop<'a, S, T, U>(conn: Arc<S>, state: State) -> !
    where T: IrcRead, U: IrcWrite, S: ServerExt<'a, T, U> + Sized {
    let has_work = state.has_work;
    let cfg = state.cfg;
    let requests = state.requests;
    loop {
        let work;
        {
            has_work.acquire();
            let mut rvec = requests.lock().unwrap();
            work = rvec.pop_front();
            std::mem::drop(rvec);
        }

        if let Some(work) = work {
            let result = state.evaluators.get(&work.language).unwrap()
                .eval(&work.code, &cfg.sandbox_dir, cfg.playpen_timeout);

            let (result, err) = match result {
                Ok(x) => (x, false),
                Err(x) => (x, true)
            };
            let result = util::wrap_output(&result,
                                     if work.is_channel { cfg.max_channel_line_len }
                                     else { 425 });

            // TODO: gist the overflow
            let max_lines = if work.is_channel { cfg.max_channel_lines } else { cfg.max_priv_lines };
            let result = util::truncate_output(result, max_lines);

            let dest = if work.is_channel { &work.target } else { &work.sender };
            for line in result.1.iter() {
                let line = format!("{}{}", if work.is_channel && !err { &cfg.chan_output_prefix as &str } else { "" }, line);
                send_msg!(conn, dest, &line);
            }
            if result.0 {
                send_msg!(conn, dest, "(output truncated)");
            }
        }
    }
}

fn parse_msg(message: &Message) -> Option<Req> {
    let sender = message.get_source_nickname().unwrap_or("");
    if let Ok(Command::PRIVMSG(target, message)) = message.into() {
        let in_channel = target.starts_with('#');
        if !in_channel && message.contains('\x01') {
            // we don't accept code via CTCP
            return None;
        }

        let tok: Vec<&str> = message.trim().splitn(2, '>').collect();
        match tok.len() {
            2 => (),
            _ => return None
        };
        let lang = match tok[0].parse::<eval::Lang>() {
            Ok(x) => x,
            Err(_) => return None
        };
        Some(Req { 
            is_channel: in_channel,
            sender: sender.to_owned(),
            target: target.to_owned(),
            code: tok[1].to_owned(),
            language: lang
        })
    } else {
        None
    }
}

fn main() {
    let config = match cfg::read("evalbot.toml") {
        Ok(x) => x,
        Err(x) => panic!("could not read config; {}", x)
    };
    println!("read config: {:?}", config);

    let conn = Arc::new(IrcServer::from_config(config.irc_config.clone()).unwrap());
    let config = Arc::new(config);
    let evalreqs = Arc::new(Mutex::new(VecDeque::new()));
    let has_work = Arc::new(Semaphore::new(0));
    
    let mut evaluators = HashMap::new();
    evaluators.insert(eval::Lang::Rust, eval::evaluator(eval::Lang::Rust, &config.sandbox_dir));
    evaluators.insert(eval::Lang::RustRaw, eval::evaluator(eval::Lang::RustRaw, &config.sandbox_dir));
    evaluators.insert(eval::Lang::Python, eval::evaluator(eval::Lang::Python, &config.sandbox_dir));
    evaluators.insert(eval::Lang::CSharp, eval::evaluator(eval::Lang::CSharp, &config.sandbox_dir));    
    let evaluators = Arc::new(evaluators);

    let state = State {
        cfg: config,
        requests: evalreqs,
        has_work: has_work,
        evaluators: evaluators
    };
    for _ in 0..state.cfg.eval_threads {
        let conn = conn.clone();
        let state = state.clone();
        thread::spawn(move || { evaluate_loop(conn, state); });
    }
    loop {
        conn.identify().unwrap();
        for maybe_msg in conn.iter() {
            let msg = match maybe_msg {
                Ok(x) => x,
                Err(x) => {
                    println!("{}, reconnecting", x);
                    break
                }
            };

            let req = parse_msg(&msg);
            if let Some(x) = req {
                println!("{} @ {} {:?}: {}", x.sender, x.target, x.language, x.code);
                state.requests.lock().unwrap().push_back(x); // if mutex is poisoned, just bail
                state.has_work.release();
            }
        }
        conn.reconnect().unwrap();
    }
}
