extern crate evalbotlib as backend;
extern crate serde;
#[macro_use] extern crate serde_derive;
#[macro_use] extern crate log;
extern crate toml;
extern crate futures;
extern crate tokio;
extern crate telebot;
extern crate env_logger;

use backend::{EvalService, Language, util};

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::borrow::Cow;

use futures::{Future, Stream, IntoFuture};
use telebot::RcBot;
use telebot::functions::*;
use telebot::objects::*;
use futures::future::Either;

macro_rules! nullify_future {
    ($task:expr, $fut:expr) => ($fut.map(|_| ())
        .or_else(|e| Ok(error!("error {}: {}", $task, e))));
}

static WHITELIST_FILENAME: &'static str = "tgwhitelist.toml";

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct TgCfg {
    owners: HashSet<String>,
    msg_owner_id: Option<i64>,
    bot_id: String,
    lang_subst: HashMap<String, String>
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct TgWhitelist {
    priv_enabled: bool,
    group_enabled: bool,
    allowed: HashSet<i64>,
    blocked: HashSet<i64>
}

impl TgWhitelist {
    fn priv_ok(&self, id: i64) -> bool {
        (!self.priv_enabled || self.allowed.contains(&id)) && !self.blocked.contains(&id)
    }

    fn group_ok(&self, id: i64) -> bool {
        (!self.group_enabled || self.allowed.contains(&id)) && !self.blocked.contains(&id)
    }

    fn allow(&mut self, id: i64) {
        self.allowed.insert(id);
    }

    fn unallow(&mut self, id: i64) {
        self.allowed.remove(&id);
    }

    fn block(&mut self, id: i64) {
        self.blocked.insert(id);
    }

    fn unblock(&mut self, id: i64) {
        self.blocked.remove(&id);
    }

    fn save<'a, 'b>(&'a self, path: &'static str) -> impl Future<Item = (), Error = ()> + 'b {
        nullify_future!("saving whitelist", util::encode(self, path))
    }
}

struct TgSvc {
    config: TgCfg,
    whitelist: RwLock<TgWhitelist>,
    service: EvalService,
}

fn telegram_wrap_result(s: &str, group: bool) -> String {
    if s.is_empty() {
        "no output".to_owned()
    } else {
        let mut r = "<pre>".to_owned();
        let input = s.as_bytes();
        let mut cut_input = String::from_utf8_lossy(&input[..512.min(input.len())]);
        if group {
            cut_input = Cow::Owned(cut_input.lines().take(10).collect::<Vec<_>>().join("\n"));
        }
        r.push_str(&cut_input
            .replace(|c: char| c =='\u{FFFD}' || (c.is_control() && c != '\n' && c != '\t'), "")
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;"));
        r.push_str("</pre>");
        if cut_input.len() + 1 // we also cut off the trailing \n
            < input.len() {
            r.push_str("... (truncated)");
        }
        r
    }
}

fn is_from_owner(msg: &Message, tgsvc: &TgSvc) -> bool {
    msg.from.as_ref().and_then(|u| u.username.as_ref().map(|un| tgsvc.is_owner(un))).unwrap_or(false)
}

fn handle_update((tgbot, update): (RcBot, Update), tgsvc: &Arc<TgSvc>) -> Result<(), ()> {
    if let Some((chat, Some(new_user))) = update.message.map(|m| (m.chat, m.new_chat_member)) {
        let chat_id = chat.id;
        if let (Ok(id), Ok(wl)) = (tgbot.inner.id.read(), tgsvc.whitelist.read()) {
            if *id == Some(new_user.id) && !wl.group_ok(chat_id) {
                tokio::spawn(nullify_future!("leaving group",
                    tgbot.message(chat_id, format!("You or this group is not on the whitelist. Seek help. ID: {}", chat_id)).send()
                        .and_then(move |(tgbot, _)| tgbot.leave_chat(chat_id).send())));
            }
        }
    }
    Ok(())
}

fn handle_eval(tgsvc: &Arc<TgSvc>, tgbot: RcBot, msg: Message, lang: &Arc<Language>, is_hash: bool)
    -> impl Future<Item = (), Error = ()> {
    let chat_id = msg.chat.id;
    let group = msg.chat.kind != "private";
    if let Ok(wl) = tgsvc.whitelist.read() {
        if group && !wl.group_ok(chat_id)
            || !group && !wl.priv_ok(chat_id) {
            tokio::spawn(nullify_future!("sending message",
                tgbot.message(chat_id, format!("You or this group is not on the whitelist. Seek help. ID: {}", chat_id)).send()));
            return Ok(()).into_future();
        }
    } else {
        error!("Failed to acquire RwLock");
        tokio::spawn(nullify_future!("sending message",
                tgbot.message(chat_id, "Internal error occurred".to_owned()).send()));
        return Ok(()).into_future();
    }

    let no_limit = is_hash && is_from_owner(&msg, tgsvc);
    let msg_id = msg.message_id;
    info!("({}) evaluating from {:?}: {:?}", msg_id, msg.from, msg.text.as_ref().map(|x| x.as_str()).unwrap_or(""));
    let code = msg.text.map(|x| {
        let mut r = x.trim_left().to_owned();
        r.push_str("\n");
        r
    }).unwrap_or_else(|| "".to_owned());
    tokio::spawn(nullify_future!("sending message",
        lang.eval(code, if no_limit { Some(0) } else { None }, Some(format!("tg{}", chat_id)))
            .then(move |e| {
                info!("({}) result: {:?}", msg_id, e.as_ref().map(|x| x.as_str()).unwrap_or(""));
                tgbot.message(chat_id, telegram_wrap_result(&e.unwrap_or_else(|x| x), group))
                    .parse_mode(ParseMode::HTML)
                    .reply_to_message_id(msg_id)
                    .send()
            })));

    Ok(()).into_future()
}

#[derive(Clone, Copy)]
enum WhitelistToggleOp {
    TogglePrivate,
    ToggleGroup
}

#[derive(Clone, Copy)]
enum WhitelistModOp {
    Allow,
    Unallow,
    Block,
    Unblock
}

fn handle_whitelist_toggle(me: &Arc<TgSvc>, tgbot: RcBot, msg: Message, op: WhitelistToggleOp)
    -> impl Future<Item = (), Error = ()> {
    if !is_from_owner(&msg, me) {
        return Either::A(Ok(()).into_future());
    }

    let resp = match me.whitelist.write() {
        Ok(mut wl) => match op {
            WhitelistToggleOp::TogglePrivate => {
                wl.priv_enabled = !wl.priv_enabled;
                tokio::spawn(wl.save(WHITELIST_FILENAME));
                format!("Private whitelist enabled: {}", wl.priv_enabled)
            }
            WhitelistToggleOp::ToggleGroup => {
                wl.group_enabled = !wl.group_enabled;
                tokio::spawn(wl.save(WHITELIST_FILENAME));
                format!("Group whitelist enabled: {}", wl.group_enabled)
            }
        }
        Err(err) => {
            error!("error while acquiring RwLock: {}", err);
            "error acquiring RwLock".to_owned()
        }
    };
    Either::B(nullify_future!("sending message", tgbot.message(msg.chat.id, resp)
        .reply_to_message_id(msg.message_id)
        .send()))
}

fn handle_whitelist_mod(me: &Arc<TgSvc>, tgbot: RcBot, msg: Message, op: WhitelistModOp)
    -> impl Future<Item = (), Error = ()> {
    if !is_from_owner(&msg, me) {
        return Either::A(Ok(()).into_future());
    }

    let arg = msg.text.as_ref().and_then(|t| t.trim().split_whitespace()
        .nth(0).and_then(|arg| arg.parse().ok()));
    let resp = match (arg, me.whitelist.write()) {
        (Some(id), Ok(mut wl)) => match op {
            WhitelistModOp::Allow => {
                wl.allow(id);
                tokio::spawn(wl.save(WHITELIST_FILENAME));
                format!("Allowed {}", id)
            }
            WhitelistModOp::Unallow => {
                wl.unallow(id);
                tokio::spawn(wl.save(WHITELIST_FILENAME));
                format!("Unallowed {}", id)
            }
            WhitelistModOp::Block => {
                wl.block(id);
                tokio::spawn(wl.save(WHITELIST_FILENAME));
                format!("Blocked {}", id)
            }
            WhitelistModOp::Unblock => {
                wl.unblock(id);
                tokio::spawn(wl.save(WHITELIST_FILENAME));
                format!("Unblocked {}", id)
            }
        }
        (None, _) => "Invalid ID".to_owned(),
        (_, Err(err)) => {
            error!("error while acquiring RwLock: {}", err);
            "error acquiring RwLock".to_owned()
        }
    };

    Either::B(nullify_future!("sending message", tgbot.message(msg.chat.id, resp)
        .reply_to_message_id(msg.message_id)
        .send()))
}

fn handle_leave(me: &Arc<TgSvc>, tgbot: RcBot, msg: Message)
    -> impl Future<Item = (), Error = ()> {
    if !is_from_owner(&msg, me) {
        return Either::A(Ok(()).into_future());
    }

    let arg = msg.text.as_ref().and_then(|t| t.trim().split_whitespace()
        .nth(0).and_then(|arg| arg.parse().ok()));
    let resp = match arg {
        Some(id) => {
            tokio::spawn(nullify_future!("leaving group", tgbot.leave_chat(id).send()));
            "OK"
        }
        None => "Invalid ID"
    }.to_owned();

    Either::B(nullify_future!("sending message", tgbot.message(msg.chat.id, resp)
        .reply_to_message_id(msg.message_id)
        .send()))
}

impl TgSvc {
    fn run() -> impl Future<Item = (), Error = ()> {
        let cfgf = util::decode::<TgCfg, _>("evalbot.tg.toml")
            .map(|cfg| {
                debug!("Loaded config: {:?}", cfg);
                cfg
            })
            .map_err(|e| {
                error!("failed to read evalbot.tg.toml: {}", e);
            });
        let wlf = util::decode::<TgWhitelist, _>(WHITELIST_FILENAME).or_else(|e| {
            warn!("failed to read whitelist: {}; using empty whitelist", e);
            Ok(TgWhitelist {
                priv_enabled: false,
                group_enabled: false,
                allowed: HashSet::new(),
                blocked: HashSet::new(),
            }).into_future()
        });
        cfgf.join(wlf).join(EvalService::from_toml_file("evalbot.toml")
            .map_err(|e| {
                error!("failed to read evalbot.toml: {}", e);
            }))
            .map(|((cfg, wl), es)| TgSvc {
                config: cfg,
                whitelist: RwLock::new(wl),
                service: es
            })
            .and_then(TgSvc::handle)
    }

    fn handle(self) -> impl Future<Item = (), Error = ()> {
        let bot = RcBot::new(&self.config.bot_id).expect("Failed to initialise Telegram bot");
        let me = Arc::new(self);
        bot.resolve_name();

        macro_rules! handle {
            ($cmd:expr, $handler:ident $(,$params:expr)*) => {{
                let me = me.clone();
                bot.register(bot.new_cmd($cmd)
                    .map_err(|e| error!("error in command processing: {}", e))
                    .and_then(move |(tgbot, msg)| $handler(&me, tgbot, msg $(,$params)*)));
            }};
        }

        for (name, lang) in me.service.langs() {
            {
                let lang = lang.clone();
                handle!(name, handle_eval, &lang, false);
            }
            {
                let lang = lang.clone();
                handle!(&format!("{}#", name), handle_eval, &lang, true);
            }
        }

        handle!("privwl", handle_whitelist_toggle, WhitelistToggleOp::TogglePrivate);
        handle!("groupwl", handle_whitelist_toggle, WhitelistToggleOp::ToggleGroup);
        handle!("allow", handle_whitelist_mod, WhitelistModOp::Allow);
        handle!("unallow", handle_whitelist_mod, WhitelistModOp::Unallow);
        handle!("block", handle_whitelist_mod, WhitelistModOp::Block);
        handle!("unblock", handle_whitelist_mod, WhitelistModOp::Unblock);
        handle!("leave", handle_leave);

        bot.get_stream()
            .map_err(|e| error!("{}", e))
            .for_each(move |tuple| handle_update(tuple, &me))
            .into_future()
    }

    fn is_owner(&self, name: &str) -> bool {
        self.config.owners.contains(name)
    }
}

fn main() {
    env_logger::init();
    tokio::run(TgSvc::run());
}
