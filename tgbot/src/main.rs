use evalbotlib::{util, EvalService, Language};

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::StreamExt;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use telegram_bot::*;
use tokio::sync::RwLock;

static WHITELIST_FILENAME: &'static str = "tgwhitelist.toml";

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct TgCfg {
    owners: HashSet<i64>,
    msg_owner_id: Option<i64>,
    bot_id: String,
    lang_subst: HashMap<String, String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct TgWhitelist {
    priv_enabled: bool,
    group_enabled: bool,
    allowed: HashSet<i64>,
    blocked: HashSet<i64>,
}

impl TgWhitelist {
    fn priv_ok(&self, id: ChatId) -> bool {
        (!self.priv_enabled || self.allowed.contains(&id.into()))
            && !self.blocked.contains(&id.into())
    }

    fn group_ok(&self, id: ChatId) -> bool {
        (!self.group_enabled || self.allowed.contains(&id.into()))
            && !self.blocked.contains(&id.into())
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

    async fn save<'a>(&'a self, path: &'static str) {
        drop(util::encode(self, path).await);
    }
}

struct TgSvc {
    config: TgCfg,
    whitelist: RwLock<TgWhitelist>,
    service: EvalService,
    api: Api,
    bot_user: User,
}

fn telegram_wrap_result(s: &str, group: bool) -> String {
    // FIXME configurable max-lines and max-bytes
    if s.is_empty() {
        "no output".to_owned()
    } else {
        let mut r = "<pre>".to_owned();
        let input = s.as_bytes();
        let mut cut_input = String::from_utf8_lossy(&input[..512.min(input.len())]);
        if group {
            cut_input = Cow::Owned(cut_input.lines().take(10).collect::<Vec<_>>().join("\n"));
        }
        r.push_str(
            &cut_input
                .replace(
                    |c: char| c == '\u{FFFD}' || (c.is_control() && c != '\n' && c != '\t'),
                    "",
                )
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;"),
        );
        r.push_str("</pre>");
        if cut_input.len() + 1 // we also cut off the trailing \n
            < input.len()
        {
            r.push_str("... (truncated)");
        }
        r
    }
}

fn is_from_owner(msg: &Message, tgsvc: &TgSvc) -> bool {
    tgsvc.is_owner(msg.from.to_user_id())
}

async fn verify_allowed(chat: &MessageChat, tgsvc: &Arc<TgSvc>) -> Result<(), ()> {
    let group = if let MessageChat::Private(_) = chat {
        true
    } else {
        false
    };
    let wl = tgsvc.whitelist.read().await;
    let chat_id = chat.id();
    if group && !wl.group_ok(chat_id) || !group && !wl.priv_ok(chat_id) {
        let send_message = tgsvc.api.send(SendMessage::new(
            &chat,
            format!(
                "You or this group is not on the whitelist. Seek help. ID: {}",
                chat_id
            ),
        ));
        let leave_group = tgsvc.api.send(LeaveChat::new(&chat));
        tokio::spawn(async move {
            drop(send_message.await.map_err(|e| {
                warn!("failed to send message: {}", e);
            }));
            drop(leave_group.await.map_err(|e| {
                warn!("failed to leave group {}: {}", chat_id, e);
            }));
        });
        Err(())
    } else {
        Ok(())
    }
}

async fn handle_update(update: Update, tgsvc: &Arc<TgSvc>) -> Result<(), ()> {
    let message = match update {
        Update {
            kind: UpdateKind::Message(message),
            ..
        } => message,
        _ => return Ok(()),
    };

    match message.kind {
        MessageKind::NewChatMembers { data } => {
            handle_new_chat_members(message.chat, data, tgsvc).await?
        }
        MessageKind::Text { .. } => handle_message(message, tgsvc).await?,
        _ => (),
    };

    Ok(())
}

async fn handle_new_chat_members(
    chat: MessageChat,
    new_members: Vec<User>,
    tgsvc: &Arc<TgSvc>,
) -> Result<(), ()> {
    if new_members.iter().any(|u| u.id == tgsvc.bot_user.id) {
        verify_allowed(&chat, tgsvc).await?;
    }

    Ok(())
}

async fn handle_message(message: Message, tgsvc: &Arc<TgSvc>) -> Result<(), ()> {
    let text = if let MessageKind::Text { ref data, .. } = message.kind {
        data
    } else {
        return Ok(());
    };

    if !text.starts_with("/") {
        return Ok(());
    }

    let cmd = if let Some(cmd) = text.split_whitespace().nth(0) {
        cmd
    } else {
        return Ok(());
    };

    let args = cmd[cmd.len()..].trim_start();

    match cmd {
        "/privwl" => {
            handle_whitelist_toggle(tgsvc, &message, WhitelistToggleOp::TogglePrivate).await?
        }
        "/groupwl" => {
            handle_whitelist_toggle(tgsvc, &message, WhitelistToggleOp::ToggleGroup).await?
        }
        "/allow" => handle_whitelist_mod(tgsvc, &message, args, WhitelistModOp::Allow).await?,
        "/unallow" => handle_whitelist_mod(tgsvc, &message, args, WhitelistModOp::Unallow).await?,
        "/block" => handle_whitelist_mod(tgsvc, &message, args, WhitelistModOp::Block).await?,
        "/unblock" => handle_whitelist_mod(tgsvc, &message, args, WhitelistModOp::Unblock).await?,
        "/leave" => handle_leave(tgsvc, &message, args).await?,
        _ => {
            let (cmd, is_hash) = if cmd.ends_with('#') {
                (&cmd[..cmd.len() - 1], true)
            } else {
                (cmd, false)
            };
            if let Some((_, lang)) = tgsvc.service.langs().find(|(name, _)| *name == cmd) {
                handle_eval(tgsvc, &message, args, lang, is_hash).await?;
            }
        }
    }

    Ok(())
}

async fn handle_eval(
    tgsvc: &Arc<TgSvc>,
    msg: &Message,
    args: &str,
    lang: &Arc<Language>,
    is_hash: bool,
) -> Result<(), ()> {
    verify_allowed(&msg.chat, tgsvc).await?;
    let no_limit = is_hash && is_from_owner(&msg, tgsvc);
    let is_group = if let MessageChat::Private(_) = msg.chat {
        false
    } else {
        true
    };
    let chat_id = msg.chat.id();
    let msg_id = msg.id;
    info!("({}) evaluating from {:?}: {:?}", msg_id, msg.from, args);
    let code = {
        let mut r = args.to_owned();
        r.push_str("\n");
        r
    };

    let eval_result = lang
        .eval(
            code,
            if no_limit { Some(0) } else { None },
            Some(format!("tg{}", chat_id)),
        )
        .await;
    let ok = eval_result.is_ok();
    info!(
        "({}) result: {:?}",
        msg_id,
        eval_result.as_ref().map(|x| x.as_str()).unwrap_or("")
    );
    let mut request = SendMessage::new(
        &msg.from,
        eval_result
            .map(|r| telegram_wrap_result(&r, is_group))
            .unwrap_or_else(|e| e),
    );
    request.reply_to(msg);
    if ok {
        request.parse_mode(ParseMode::Html);
    }
    tokio::spawn(tgsvc.api.send(request));

    Ok(())
}

#[derive(Clone, Copy)]
enum WhitelistToggleOp {
    TogglePrivate,
    ToggleGroup,
}

#[derive(Clone, Copy)]
enum WhitelistModOp {
    Allow,
    Unallow,
    Block,
    Unblock,
}

async fn handle_whitelist_toggle(
    tgsvc: &Arc<TgSvc>,
    msg: &Message,
    op: WhitelistToggleOp,
) -> Result<(), ()> {
    if !is_from_owner(&msg, tgsvc) {
        return Ok(());
    }

    let mut wl = tgsvc.whitelist.write().await;
    let resp = match op {
        WhitelistToggleOp::TogglePrivate => {
            wl.priv_enabled = !wl.priv_enabled;
            wl.save(WHITELIST_FILENAME).await;
            format!("Private whitelist enabled: {}", wl.priv_enabled)
        }
        WhitelistToggleOp::ToggleGroup => {
            wl.group_enabled = !wl.group_enabled;
            wl.save(WHITELIST_FILENAME).await;
            format!("Group whitelist enabled: {}", wl.group_enabled)
        }
    };
    tokio::spawn(tgsvc.api.send(SendMessage::new(&msg.chat, resp)));
    Ok(())
}

async fn handle_whitelist_mod(
    tgsvc: &Arc<TgSvc>,
    msg: &Message,
    args: &str,
    op: WhitelistModOp,
) -> Result<(), ()> {
    if !is_from_owner(&msg, tgsvc) {
        return Ok(());
    }

    let arg = args
        .trim()
        .split_whitespace()
        .nth(0)
        .and_then(|arg| arg.parse().ok());
    let mut wl = tgsvc.whitelist.write().await;
    let resp = if let Some(id) = arg {
        match op {
            WhitelistModOp::Allow => {
                wl.allow(id);
                wl.save(WHITELIST_FILENAME).await;
                format!("Allowed {}", id)
            }
            WhitelistModOp::Unallow => {
                wl.unallow(id);
                wl.save(WHITELIST_FILENAME).await;
                format!("Unallowed {}", id)
            }
            WhitelistModOp::Block => {
                wl.block(id);
                wl.save(WHITELIST_FILENAME).await;
                format!("Blocked {}", id)
            }
            WhitelistModOp::Unblock => {
                wl.unblock(id);
                wl.save(WHITELIST_FILENAME).await;
                format!("Unblocked {}", id)
            }
        }
    } else {
        "Invalid ID".to_owned()
    };

    tokio::spawn(tgsvc.api.send(SendMessage::new(&msg.chat, resp)));
    Ok(())
}

async fn handle_leave(tgsvc: &Arc<TgSvc>, msg: &Message, args: &str) -> Result<(), ()> {
    if !is_from_owner(&msg, tgsvc) {
        return Ok(());
    }

    let arg: Option<i64> = args
        .trim()
        .split_whitespace()
        .nth(0)
        .and_then(|arg| arg.parse().ok());
    let (maybe_leave_chat, resp) = match arg {
        Some(id) => (Some(tgsvc.api.send(LeaveChat::new(ChatId::from(id)))), "OK"),
        None => (None, "Invalid ID"),
    };

    let send_response = tgsvc.api.send(SendMessage::new(&msg.chat, resp));

    tokio::spawn(async move {
        if let Some(fut) = maybe_leave_chat {
            drop(fut.await.map_err(|e| {
                warn!("failed to leave group: {}", e);
            }));
        }
        drop(send_response.await.map_err(|e| {
            warn!("failed to send message: {}", e);
        }));
    });
    Ok(())
}

/*
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

        handle!(
            "privwl",
            handle_whitelist_toggle,
            WhitelistToggleOp::TogglePrivate
        );
        handle!(
            "groupwl",
            handle_whitelist_toggle,
            WhitelistToggleOp::ToggleGroup
        );
        handle!("allow", handle_whitelist_mod, WhitelistModOp::Allow);
        handle!("unallow", handle_whitelist_mod, WhitelistModOp::Unallow);
        handle!("block", handle_whitelist_mod, WhitelistModOp::Block);
        handle!("unblock", handle_whitelist_mod, WhitelistModOp::Unblock);
        handle!("leave", handle_leave);
*/

impl TgSvc {
    async fn run() -> Result<(), ()> {
        let cfg = util::decode::<TgCfg, _>("evalbot.tg.toml")
            .await
            .map(|cfg| {
                debug!("Loaded config: {:?}", cfg);
                cfg
            })
            .map_err(|e| {
                error!("failed to read evalbot.tg.toml: {}", e);
            })?;
        let wl = util::decode::<TgWhitelist, _>(WHITELIST_FILENAME)
            .await
            .or_else(|e| {
                warn!("failed to read whitelist: {}; using empty whitelist", e);
                Ok(TgWhitelist {
                    priv_enabled: false,
                    group_enabled: false,
                    allowed: HashSet::new(),
                    blocked: HashSet::new(),
                })
            })?;

        let api = Api::new(&cfg.bot_id);
        let bot_user = api.send(GetMe).await.map_err(|e| {
            error!("failed to get bot's user info: {}", e);
        })?;
        let service = EvalService::from_toml_file("evalbot.toml")
            .await
            .map_err(|e| {
                error!("failed to read evalbot.toml: {}", e);
            })?;
        TgSvc {
            api: Api::new(&cfg.bot_id),
            config: cfg,
            whitelist: RwLock::new(wl),
            service: service,
            bot_user: bot_user,
        }
        .handle()
        .await;
        Ok(())
    }

    async fn handle(self) {
        let me = Arc::new(self);

        let mut stream = me.api.stream();
        while let Some(update) = stream.next().await {
            match update {
                Ok(update) => drop(handle_update(update, &me).await),
                Err(error) => error!("received error: {:?}", error),
            }
        }
    }

    fn is_owner(&self, id: UserId) -> bool {
        self.config.owners.contains(&id.into())
    }
}

#[tokio::main]
async fn main() {
    env_logger::init();
    drop(TgSvc::run().await);
}
