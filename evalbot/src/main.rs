#![feature(plugin)]
#![plugin(clippy)]

extern crate irc as irclib;
extern crate evalbotlib as backend;
extern crate rustc_serialize;

use std::thread;
use std::time::Duration;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicBool, Ordering};

use backend::{EvalSvc, EvalSvcCfg, Response, util};

use self::irclib::client::server::NetIrcServer;
use self::irclib::client::prelude as ircp;
use self::irclib::client::prelude::{Server, ServerExt};

macro_rules! send_msg {
    ($conn:expr, $dest:expr, $line:expr) => {
        if let Err(x) = $conn.send_privmsg($dest, $line) {
            println!("failed to send message: {}", x)
        }
    };
}

type EvalWorkerSender = mpsc::Sender<(IrcMessage, SendMsgFnBox)>;
type EvalWorkerReceiver = mpsc::Receiver<(IrcMessage, SendMsgFnBox)>;
type SendMsgFnBox = Box<Fn(&str, &str) + Send>;

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
struct IrcCfg {
    output: OutputCfg,
    command_prefix: String,
    owners: Vec<String>,
    irc_networks: Vec<ircp::Config>
}

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
struct OutputCfg {
    chan_output_prefix: String,
    max_channel_lines: usize,
    max_channel_line_len: usize,
    max_priv_lines: usize
}

fn main() {
    let config = match util::decode::<IrcCfg>("evalbot.irc.toml") {
        Ok(x) => x,
        Err(x) => {
            println!("failed to read evalbot.irc.toml: {:?}", x);
            return;
        }
    };
    let quit_handle = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();

    {
        let svc = match start_evalsvc() {
            Ok(x) => x,
            Err(()) => return,
        };
        let ocfg = config.output.clone();
        thread::spawn(move || eval_worker(svc, rx, ocfg));
    }

    for (cfg, jh) in config.irc_networks
                           .iter()
                           .filter_map(|cfg| {
                               match ircp::IrcServer::from_config(cfg.clone()) {
                                   Ok(conn) => Some((cfg, conn)),
                                   Err(ref err) => {
                                       println!("failed to create IRC connection from Config {:#?}: {:?}", cfg, err);
                                       None
                                   }
                               }
                           })
                           .enumerate()
                           .map(|(k, (cfg, nwk))| {
                               (cfg,
                                start_worker(nwk,
                                             tx.clone(),
                                             config.owners.clone(),
                                             config.command_prefix.clone(),
                                             quit_handle.clone(),
                                             format!("{}", k)))
                           })
                           .collect::<Vec<_>>() {
        util::ignore(jh.join());
        println!("disconnected as {:?} from {:?}", cfg.nickname, cfg.server);
    }
}

fn start_evalsvc() -> Result<EvalSvc, ()> {
    let evalcfg = match util::decode::<EvalSvcCfg>("evalbot.toml") {
        Ok(x) => x,
        Err(x) => {
            println!("failed to read evalbot.toml: {:?}", x);
            return Err(());
        }
    };
    Ok(EvalSvc::new(evalcfg))
}

fn start_worker(conn: NetIrcServer,
    svc: EvalWorkerSender,
    owners: Vec<String>,
    cmd_prefix: String,
    quit_handle: Arc<AtomicBool>,
    conn_hash: String)
                -> thread::JoinHandle<()> {
    thread::spawn(move || {
        worker(conn, svc, owners, cmd_prefix, quit_handle, conn_hash);
    })
}

#[derive(Clone, Debug)]
struct MessageSource {
    chan: Option<String>,
    sender: String,
    conn_hash: String
}

impl MessageSource {
    fn reply_to(&self) -> &str {
        if let Some(ref chan) = self.chan { chan } else { &self.sender }
    }

    fn is_chan(&self) -> bool {
        self.chan.is_some()
    }
}

#[derive(Debug)]
struct IrcMessage {
    sender: MessageSource,
    data: MessageData
}

#[derive(Debug)]
enum MessageData {
    EvalReq {
        lang: String,
        code: String,
        timeout: bool
    },
    Quit,
    Rehash,
    Restart {
        lang: String
    }
}

unsafe impl Sync for IrcMessage {}

fn parse_msg(conn_hash: &str, message: &ircp::Message, owners: &[String], cmd_prefix: &str) -> Option<IrcMessage> {
    let sender = message.source_nickname().unwrap_or("").to_owned();
    if let ircp::Command::PRIVMSG(ref target, ref message) = message.command {
        let target = target.to_owned();
        let is_owner = owners.iter().any(|x| *x == sender);
        let sender = {
            let in_channel = target.starts_with('#');
            if !in_channel && message.contains('\x01') {
                // ignore CTCP
                return None;
            }

            MessageSource {
                chan: if in_channel { Some(target) } else { None },
                sender: sender,
                conn_hash: conn_hash.to_owned()
            }
        };

        if is_owner {
            if let Some(x) = try_cmd(&message, cmd_prefix) {
                return Some(IrcMessage { sender: sender, data: x });
            }
        }

        let tok: Vec<&str> = message.trim().splitn(2, '>').collect();
        if tok.len() == 2 {
            return Some(IrcMessage {
                sender: sender,
                data: MessageData::EvalReq { timeout: true, lang: tok[0].to_owned(), code: tok[1].to_owned() }
            });
        } else if !is_owner {
            return None;
        }

        let tok: Vec<&str> = message.trim().splitn(2, '#').collect();
        if tok.len() == 2 {
            Some(IrcMessage {
                sender: sender,
                data: MessageData::EvalReq { timeout: false, lang: tok[0].to_owned(), code: tok[1].to_owned() }
            })
        } else {
            None
        }
    } else {
        None
    }
}

fn try_cmd(msg: &str, cmd_prefix: &str) -> Option<MessageData> {
    if let Some((cmd, args)) = parse_cmd(msg, cmd_prefix) {
        match &cmd as &str {
            "quit" => Some(MessageData::Quit),
            "rehash" => Some(MessageData::Rehash),
            "restart" if !args.is_empty() => Some(MessageData::Restart { lang: args[0].to_owned() }),
            _ => None,
        }
    } else {
        None
    }
}

fn parse_cmd(msg: &str, cmd_prefix: &str) -> Option<(String, Vec<String>)> {
    let tok: Vec<&str> = match msg.splitn(2, cmd_prefix).nth(1).map(|x| x.split(' ').collect()) {
        Some(x) => x,
        None => return None,
    };

    if tok.is_empty() {
        return None;
    }

    Some((tok[0].to_owned(),
          tok.into_iter().skip(1).map(|x| x.to_owned()).collect::<Vec<String>>()))
}

fn worker(conn: NetIrcServer,
    tx: EvalWorkerSender,
    owners: Vec<String>,
    cmd_prefix: String,
    quit_handle: Arc<AtomicBool>,
    conn_hash: String) {
    'connection: loop {
        println!("connecting to to {:?} as {:?}", conn.config().server, conn.config().nickname);
        while let Err(x) = conn.identify() {
            println!("error while identify()ing; retrying: {:?}", x);
            thread::sleep(Duration::new(1, 0));
        }
        println!("connected to {:?} as {:?}", conn.config().server, conn.config().nickname);
        for msg in conn.iter() {
            if quit_handle.load(Ordering::Relaxed) {
                break 'connection;
            }
            let msg = match msg {
                Ok(x) => x,
                Err(_) => continue,
            };

            if let Some(msg) = parse_msg(&conn_hash, &msg, &owners, &cmd_prefix) {
                let conn = conn.clone();
                println!("M: {:?}", msg);
                match msg.data {
                    MessageData::EvalReq { .. } | MessageData::Rehash | MessageData::Restart { .. } => {
                        util::ignore(tx.send((msg, Box::new(move |to, r| send_msg!(conn, to, r)))))
                    }
                    MessageData::Quit => {
                        quit_handle.store(true, Ordering::Relaxed);
                        break 'connection;
                    }
                }
            }
        }
    }
    util::ignore(conn.send_quit("Quitting"));
}

fn eval_worker(mut svc: EvalSvc, rx: EvalWorkerReceiver, ocfg: OutputCfg) {
    while let Ok((msg, callback)) = rx.recv() {
        macro_rules! reply {
            ($x:expr) => { callback(msg.sender.reply_to(), $x); }
        }
        match msg.data {
            MessageData::EvalReq { lang, code, timeout } => {
                let ocfg = ocfg.clone();
                let sender = msg.sender.clone();
                svc.eval(&lang,
                         code,
                         timeout,
                         Some(format!("{}{}", msg.sender.conn_hash, msg.sender.reply_to())),
                         Box::new(move |resp| respond(resp, ocfg, sender, callback)))
            }
            MessageData::Rehash => {
                if let Ok(nsvc) = start_evalsvc() {
                    reply!("rehashed");
                    svc = nsvc;
                } else {
                    reply!("read config failed; check logs");
                }
            }
            MessageData::Restart { lang } => {
                match svc.restart(&lang) {
                    Ok(_) => reply!(&format!("restarted {}", lang)),
                    Err(_) => reply!(&format!("no such language {}", lang)),
                }
            }
            _ => println!("invalid thing sent to eval_worker?"),
        };
    }
}

fn sanitise_output(input: &str, prefix: Option<&str>, max_len: usize, max_lines: usize) -> Vec<String> {
    let (mut ret, initial_lines): (Vec<String>, usize) = {
        let med = input.lines()
                       .map(|l| l.trim())
                       .filter(|l| !l.is_empty())
                       .flat_map(|l| l.split(util::LengthPattern(max_len)))
                       .collect::<Vec<_>>();

        (if let Some(prefix) = prefix {
            med.iter().take(max_lines).map(|l| prefix.to_owned() + l).collect()
        } else {
            med.iter().take(max_lines).map(|l| (*l).to_owned()).collect()
        },
         med.len())
    };
    if ret.is_empty() {
        ret.push("(ok, no output)".to_owned());
    } else if ret.len() < initial_lines {
        if let Some(ref mut l) = ret.last_mut() {
            l.push_str("... (truncated)");
        }
    }
    ret
}

#[allow(boxed_local)] // send_msg is already boxed anyway
fn respond(resp: Response, cfg: OutputCfg, to: MessageSource, send_msg: SendMsgFnBox) {
    let (response, error) = match resp {
        Response::Success(x) => (x, false),
        Response::Error(x) => (x, true),
        _ => return,
    };
    let output = sanitise_output(&response,
                                 if to.is_chan() && !error { Some(&cfg.chan_output_prefix) } else { None },
                                 if to.is_chan() { cfg.max_channel_line_len } else { 425 },
                                 if to.is_chan() { cfg.max_channel_lines } else { cfg.max_priv_lines });
    for line in &output {
        send_msg(to.reply_to(), line);
    }
}
