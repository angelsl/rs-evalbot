#![feature(semaphore)]

extern crate irc;
extern crate rustc_serialize;
extern crate toml;

mod playpen;
mod cfg;
mod util;

use irc::client::prelude::*;
use std::sync::{Arc, Mutex, Semaphore};
use std::thread;
use std::collections::VecDeque;
use cfg::EvalbotCfg;

#[derive(Debug)]
struct EvalReq {
    pub raw: bool,
    pub is_channel: bool,
    pub sender: String,
    pub target: String,
    pub code: String
}

#[cfg(not(unix))]
fn evaluate(code: &str, _: &str, _: usize) -> String {
    if let Ok(x) = code.parse::<usize>() {
        std::iter::repeat("X").take(x).collect::<String>()
    } else {
        "not a number".to_owned()
    }
}

#[cfg(unix)]
fn evaluate(code: &str, sandbox: &str, timeout: usize) -> String {
    let (stdout, stderr) = match playpen::exec(sandbox, "/usr/local/bin/evaluate.sh",
                                        &["-C","opt-level=2"],
                                        &code,
                                        timeout) {
        Ok(x) => x,
        Err(x) => return x
    };
    let stdout = stdout.replace("\u{FFFD}", "");
    let mut out = String::new();
    for line in stdout.lines() {
        out.push_str(&format!("stdout: {}\n", line));
    }
    for line in stderr.lines() {
        out.push_str(&format!("stderr: {}\n", line));
    }
    out
}

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

fn evaluate_loop<'a, S, T, U>(conn: Arc<S>, requests: Arc<Mutex<VecDeque<EvalReq>>>, has_work: Arc<Semaphore>, cfg: Arc<EvalbotCfg>) -> !
    where T: IrcRead, U: IrcWrite, S: ServerExt<'a, T, U> + Sized {
    macro_rules! send_msg {
        ($dest:expr, $line:expr) => {
            match conn.send_privmsg($dest, $line) {
                Err(x) => println!("failed to send message: {}", x),
                _ => ()
            }
        };
    }

    loop {
        has_work.acquire();
        let mut rvec = requests.lock().unwrap();
        let work = rvec.pop_front();
        std::mem::drop(rvec);
        if let Some(work) = work {
            let code = if work.raw { work.code } else { expr_to_program(&work.code) };
            let result = evaluate(&code, &cfg.sandbox_dir, cfg.playpen_timeout);
            let result = util::wrap_output(&result,
                                     if work.is_channel { cfg.max_channel_line_len }
                                     else { 425 });

            // TODO: gist the overflow
            let max_lines = if work.is_channel { cfg.max_channel_lines } else { cfg.max_priv_lines };
            let result = util::truncate_output(result, max_lines);

            let dest = if work.is_channel { &work.target } else { &work.sender };
            for line in result.1.iter() {
                let line = format!("{}{}", if work.is_channel { &cfg.chan_output_prefix as &str } else { "" }, line);
                send_msg!(dest, &line);
            }
            if result.0 {
                send_msg!(dest, "(output truncated)");
            }
        }
    }
}

fn parse_msg(message: &Message) -> Option<EvalReq> {
    let sender = message.get_source_nickname().unwrap_or("");
    if let Ok(Command::PRIVMSG(target, message)) = message.into() {
        let in_channel = target.starts_with('#');
        if !in_channel && message.contains('\x01') {
            // we don't accept code via CTCP
            return None;
        }

        let tok: Vec<&str> = message.trim().splitn(2, '>').collect();
        // in channel: rust[raw]>code...
        // in private message: [raw>]code...
        let raw = match (in_channel, tok[0]) {
            (true, "rust") => false,
            (true, "rustraw") => true,
            (false, "raw") => true,
            (false, _) => false,
            _ => return None
        };
        let code = if !raw && !in_channel {
            &message as &str
        } else {
            match tok.get(1) { Some(x) => *x, None => return None }
        };
        Some(EvalReq { raw: raw, is_channel: in_channel, sender: sender.to_owned(), target: target.to_owned(), code: code.to_owned() })
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

            if let Some(x) = parse_msg(&msg) {
                println!("{} @ {} (raw: {}): {}", x.sender, x.target, x.raw, x.code);
                evalreqs.lock().unwrap().push_back(x); // if mutex is poisoned, just bail
                has_work.release();
            }
        }
        conn.reconnect().unwrap();
    }
}
