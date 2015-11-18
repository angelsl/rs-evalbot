#![feature(semaphore)]

extern crate irc;
extern crate rustc_serialize;
extern crate toml;

mod playpen;
mod cfg;
mod util;
mod eval;

use irc::client::prelude::*;
use std::sync::{Arc, Mutex, Semaphore};
use std::thread;
use std::collections::VecDeque;

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

fn evaluate_loop<'a, S, T, U>(conn: Arc<S>, requests: Arc<Mutex<VecDeque<Req>>>, has_work: Arc<Semaphore>, cfg: Arc<EvalbotCfg>) -> !
    where T: IrcRead, U: IrcWrite, S: ServerExt<'a, T, U> + Sized {
    loop {
        has_work.acquire();
        let mut rvec = requests.lock().unwrap();
        let work = rvec.pop_front();
        std::mem::drop(rvec);
        if let Some(work) = work {
            let result = eval::eval(&work, &cfg.sandbox_dir, cfg.playpen_timeout);
            let result = util::wrap_output(&result,
                                     if work.is_channel { cfg.max_channel_line_len }
                                     else { 425 });

            // TODO: gist the overflow
            let max_lines = if work.is_channel { cfg.max_channel_lines } else { cfg.max_priv_lines };
            let result = util::truncate_output(result, max_lines);

            let dest = if work.is_channel { &work.target } else { &work.sender };
            for line in result.1.iter() {
                let line = format!("{}{}", if work.is_channel { &cfg.chan_output_prefix as &str } else { "" }, line);
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
    for _ in 0..config.eval_threads {
        let conn = conn.clone();
        let evalreqs = evalreqs.clone();
        let has_work = has_work.clone();
        let config = config.clone();
        thread::spawn(move || { evaluate_loop(conn, evalreqs, has_work, config.clone()); });
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
                evalreqs.lock().unwrap().push_back(x); // if mutex is poisoned, just bail
                has_work.release();
            }
        }
        conn.reconnect().unwrap();
    }
}
