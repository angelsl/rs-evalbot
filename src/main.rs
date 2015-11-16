#![feature(semaphore)]

extern crate irc;
extern crate toml;
extern crate rustc_serialize;

use irc::client::prelude::*;

#[derive(Clone, RustcDecodable, RustcEncodable, Default, PartialEq, Debug)]
struct EvalbotCfg {
    pub chan_output_prefix: String,
    pub max_channel_lines: usize,
    pub max_channel_line_len: usize,
    pub playpen_timeout: usize,
    pub eval_threads: usize,
    pub irc_config: Config
}

unsafe impl Send for EvalbotCfg {}

fn read_cfg() -> Result<EvalbotCfg, String> {
    use std::io::prelude::*;
    use std::fs::File;
    use rustc_serialize::Decodable;

    let mut f = try!(File::open("evalbot.toml")
                     .map_err(|x| format!("could not open evalbot.toml: {}", x)));
    let mut s = String::new();

    try!(f.read_to_string(&mut s)
         .map_err(|x| format!("could not read evalbot.toml: {}", x)));

    let value = try!(s.parse::<toml::Value>()
                     .map_err(|x| format!("could not parse evalbot.toml: {:?}", x)));

    EvalbotCfg::decode(&mut toml::Decoder::new(value))
         .map_err(|x| format!("could not decode evalbot.toml: {}", x))
}

#[derive(Debug)]
struct EvalReq {
    pub raw: bool,
    pub is_channel: bool,
    pub sender: String,
    pub target: String,
    pub code: String
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

use std::sync::{Arc, Mutex, Semaphore};
use std::thread;
use std::collections::VecDeque;

#[cfg(not(unix))]
fn evaluate(code: &str, _: usize) -> String {
    if let Ok(x) = code.parse::<usize>() {
        std::iter::repeat("X").take(x).collect::<String>()
    } else {
        "not a number".to_owned()
    }
}

#[cfg(unix)]
fn playpen_exec(command: &str, args: &[&str], input: &str, timeout: usize) -> Result<(String, String), String> {
    use std::process::{Command,Stdio};
    use std::io::Write;
    let mut child = try!(Command::new("sudo")
        .arg("playpen")
        .arg("sandbox")
        .arg("--mount-proc")
        .arg("--user=rust")
        .arg(format!("--timeout={}", timeout))
        .arg("--syscalls-file=whitelist")
        .arg("--devices=/dev/urandom:r,/dev/null:w")
        .arg("--memory-limit=128")
        .arg("--")
        .arg(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn().map_err(|x| format!("couldn't playpen_exec; {}", x)));
    if let Some(ref mut x) = child.stdin { 
        try!(x.write_all(input.as_bytes())
             .map_err(|x| format!("couldn't write to stdin; {}", x)));
    } else {
        return Err("no stdin?".to_owned());
    }
    let output = try!(child.wait_with_output().
                      map_err(|x| format!("wait_with_output failed; {}", x)));
    Ok((String::from_utf8_lossy(&output.stdout).into_owned(), 
     String::from_utf8_lossy(&output.stderr).into_owned()))
}

#[cfg(unix)]
fn evaluate(code: &str, timeout: usize) -> String {
    let code = format!(
r#"#![allow(dead_code, unused_variables)]
fn show<T: std::fmt::Debug>(e: T) {{ println!("{{:?}}", e) }}
fn main() {{
    show({{
        {}
    }});
}}"#, code);
    let (stdout, stderr) = match playpen_exec("/usr/local/bin/evaluate.sh",
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

fn wrap_output<'a>(input: &'a str, max_len: usize) -> Vec<&'a str> {
    let mut ret = vec![];
    for line in input.lines() {
        let line = line.trim();
        if line.len() <= max_len {
            ret.push(line);
        } else {
            let mut leftover = line;
            while let Some((idx, _)) = leftover.char_indices().nth(max_len) {
                let (part, after) = leftover.split_at(idx);
                ret.push(part);
                leftover = after;
            }
            if leftover.len() > 0 {
                ret.push(leftover);
            }
        }
    }
    ret
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
            let result = evaluate(&work.code, cfg.playpen_timeout);
            let result = wrap_output(&result,
                                     if work.is_channel { cfg.max_channel_line_len }
                                     else { 425 });
            // TODO: gist the overflow
            let max_lines = if work.is_channel { cfg.max_channel_lines } else { usize::max_value() };
            let max_lines = std::cmp::min(max_lines, result.len());
            let dest = if work.is_channel { &work.target } else { &work.sender };
            for line in result[0..max_lines].iter() {
                let line = format!("{}{}", if work.is_channel { &cfg.chan_output_prefix as &str } else { "" }, line);
                send_msg!(dest, &line);
            }
            if max_lines < result.len() {
                send_msg!(dest, "(output truncated)");
            }
        }
    }
}

fn main() {
    let config = match read_cfg() {
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
