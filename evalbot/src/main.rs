#![feature(semaphore)]
#![feature(plugin)]
#![plugin(clippy)]

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
use std::time::Duration;
use std::collections::{VecDeque, HashMap};

use cfg::EvalbotCfg;
use eval::Lang;

macro_rules! send_msg {
    ($conn:expr, $dest:expr, $line:expr) => {
        if let Err(x) = $conn.send_privmsg($dest, $line) {
            println!("failed to send message: {}", x)
        }
    };
}

macro_rules! send_notice {
    ($conn:expr, $dest:expr, $line:expr) => {
        if let Err(x) = $conn.send_notice($dest, $line) {
            println!("failed to send notice: {}", x)
        }
    };
}

#[derive(Debug)]
struct Req {
    pub is_channel: bool,
    pub sender: String,
    pub target: String,
    pub code: String,
    pub language: Arc<Box<Lang>>
}

#[derive(Debug)]
enum CommandResult {
    Req(Req),
    Rehash,
    RestartEvaluator(String)
}

#[derive(Debug)]
enum WorkerMessage {
    Req(Req),
    Terminate
}

#[derive(Clone)]
struct State {
    requests: Arc<Mutex<VecDeque<WorkerMessage>>>,
    has_work: Arc<Semaphore>,
    cfg: Arc<EvalbotCfg>,
    languages: Arc<HashMap<String, Arc<Box<Lang>>>>
}

impl State {
    fn send_message(&self, x: WorkerMessage) {
        self.requests.lock().unwrap().push_back(x); // if mutex is poisoned, just bail
        self.has_work.release();
    }
}

fn evaluate_loop<'a, S, T, U>(conn: Arc<S>, state: State)
    where T: IrcRead,
          U: IrcWrite,
          S: ServerExt<'a, T, U> + Sized {
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

        if let Some(WorkerMessage::Req(work)) = work {
            let result = work.language.eval(&work.code);

            let (result, err) = match result {
                Ok(x) => (x, false),
                Err(x) => (x, true),
            };

            let dest = if work.is_channel { &work.target } else { &work.sender };

            if result.is_empty() {
                send_msg!(conn, dest, "ok (no output)");
                continue;
            }

            let result = util::wrap_and_trim_output(&result,
                                                    if work.is_channel {
                                                        cfg.max_channel_line_len
                                                    } else {
                                                        425
                                                    });

            let max_lines = if work.is_channel {
                cfg.max_channel_lines
            } else {
                cfg.max_priv_lines
            };
            let (truncated, result) = util::truncate_output(result, max_lines);
            let iter = result.iter().take(if truncated {
            // truncated: we take up to the 2nd last line, as we add "... (output truncated)"
            // to the last line
                result.len() - 1
            } else {
                result.len()
            });
            let prefix = if work.is_channel && !err {
                &cfg.chan_output_prefix as &str
            } else {
                ""
            };
            for line in iter {
                let line = format!("{}{}", prefix, line);
                send_msg!(conn, dest, &line);
            }
            if truncated {
                send_msg!(conn,
                          dest,
                          &format!("{}{}... (output truncated)", prefix, result.last().unwrap()));
            }
        } else if let Some(WorkerMessage::Terminate) = work {
            println!("got rehashing message; terminating");
            break;
        }
    }
}

fn parse_command(message: &str, prefix: &str) -> Option<CommandResult> {
    let tok: Vec<&str> = match message.splitn(2, prefix)
                                      .nth(1)
                                      .map(|x| x.split(' ').collect()) {
        Some(x) => x,
        None => return None,
    };
    if tok.is_empty() {
        return None;
    }
    match tok[0] {
        "rehash" => Some(CommandResult::Rehash),
        "restart" => {
            if tok.len() < 2 {
                None
            } else {
                Some(CommandResult::RestartEvaluator(tok[1].to_owned()))
            }
        }
        _ => None,
    }
}

fn parse_msg(message: &Message, state: &State) -> Option<(CommandResult, String)> {
    let sender = message.get_source_nickname().unwrap_or("");
    if let Ok(Command::PRIVMSG(target, message)) = message.into() {
        let in_channel = target.starts_with('#');
        if !in_channel && message.contains('\x01') {
            // we don't accept code via CTCP
            return None;
        }

        if state.cfg.owners.iter().any(|x| *x == sender) {
            if let Some(cmd) = parse_command(&message, &state.cfg.command_prefix) {
                return Some((cmd, sender.to_owned()));
            }
        }

        let tok: Vec<&str> = message.trim().splitn(2, '>').collect();
        match tok.len() {
            2 => (),
            _ => return None,
        };

        let language = state.languages.get(tok[0]);
        if let Some(language) = language {
            Some((CommandResult::Req(Req {
                is_channel: in_channel,
                sender: sender.to_owned(),
                target: target.to_owned(),
                code: tok[1].to_owned(),
                language: language.clone()
            }),
                  sender.to_owned()))
        } else {
            None
        }
    } else {
        None
    }
}

fn main() {
    let irc_config = match cfg::read("evalbot.irc.toml") {
        Ok(x) => x,
        Err(x) => panic!("could not read irc config; {}", x),
    };
    println!("read irc config: {:?}", irc_config);

    let conn = Arc::new(IrcServer::from_config(irc_config).unwrap());
    let mut rehash_cfg = None;
    loop {
        while !conn.identify().is_ok() {
            thread::sleep(Duration::new(1, 0));
        }
        'connection: loop {
            let config = {
                if let Some(cfg) = rehash_cfg.take() {
                    Arc::new(cfg)
                } else {
                    Arc::new(match cfg::read::<EvalbotCfg>("evalbot.toml") {
                        Ok(x) => x,
                        Err(x) => panic!("could not read config; {}", x),
                    })
                }
            };
            println!("read config: {:?}", config);
            let evalreqs = Arc::new(Mutex::new(VecDeque::new()));
            let has_work = Arc::new(Semaphore::new(0));

            let mut languages = HashMap::new();
            for lang in config.languages.clone() {
                let short_name = lang.short_name.clone();
                let long_name = lang.long_name.clone();
                let lang_obj = Arc::new(eval::new(lang,
                                                  config.playpen_args.clone(),
                                                  config.sandbox_dir.clone(),
                                                  config.playpen_timeout));
                languages.insert(short_name, lang_obj.clone());
                languages.insert(long_name, lang_obj);
            }
            let languages = Arc::new(languages);

            let state = State {
                cfg: config,
                requests: evalreqs,
                has_work: has_work,
                languages: languages
            };

            for _ in 0..state.cfg.eval_threads {
                let conn = conn.clone();
                let state = state.clone();
                thread::spawn(move || {
                    evaluate_loop(conn, state);
                });
            }

            for maybe_msg in conn.iter() {
                let msg = match maybe_msg {
                    Ok(x) => x,
                    Err(x) => {
                        println!("{}, reconnecting", x);
                        break 'connection;
                    }
                };

                let req = parse_msg(&msg, &state);
                if let Some((x, sender)) = req {
                    println!("{:?}", (&x, &sender));
                    match x {
                        CommandResult::Req(req) => state.send_message(WorkerMessage::Req(req)),
                        CommandResult::Rehash => {
                            let maybe_rehash_cfg = cfg::read::<EvalbotCfg>("evalbot.toml");
                            match maybe_rehash_cfg {
                                Ok(cfg) => {
                                    rehash_cfg = Some(cfg);
                                    send_notice!(conn,
                                                 &sender,
                                                 "read configuration file OK, rehashing");
                                    for _ in 0..state.cfg.eval_threads {
                                        state.send_message(WorkerMessage::Terminate);
                                    }
                                    for language in state.languages.values() {
                                        language.terminate();
                                    }
                                    continue 'connection;
                                }
                                Err(x) => {
                                    send_notice!(conn,
                                                 &sender,
                                                 "error reading configuration, check console");
                                    println!("error while reading config for rehash: {}", x);
                                }
                            };
                        }
                        CommandResult::RestartEvaluator(lang) => {
                            if let Some(lang) = state.languages.get(&lang) {
                                lang.restart();
                                send_notice!(conn, &sender, "restarting evaluator");
                            } else {
                                send_notice!(conn, &sender, "invalid language name");
                            }
                        }
                    };
                }
            }
            break 'connection;
        }
        while !conn.reconnect().is_ok() {
            thread::sleep(Duration::new(1, 0));
        }
    }
}
