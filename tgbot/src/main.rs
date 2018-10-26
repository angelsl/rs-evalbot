extern crate evalbotlib as backend;
extern crate hyper;
extern crate serde;
#[macro_use] extern crate serde_derive;
extern crate serde_json;
extern crate toml;
extern crate futures;

use backend::{EvalSvc, EvalSvcCfg, Response, util};

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::sync::RwLock;

use hyper::{Body, Method, StatusCode, Server};
use futures::{Future, Stream};

macro_rules! ignore_req {
    () => {
        {
            d!(println!("ignore_req!() @ {}:{}", file!(), line!()));
            return Ok("".to_owned());
        }
    }
}

#[cfg(feature = "debugprint")]
macro_rules! d { ($x:expr) => { $x } }
#[cfg(not(feature = "debugprint"))]
macro_rules! d { ($x:expr) => {} }


static WHITELIST_FILENAME: &'static str = "tgwhitelist.toml";

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Debug)]
struct TgCfg {
    owners: HashSet<String>,
    msg_owner_id: Option<i64>,
    bot_id: String,
    lang_subst: HashMap<String, String>
}

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Debug)]
struct TgWhitelist {
    priv_enabled: bool,
    group_enabled: bool,
    allowed: HashSet<i64>,
    blocked: HashSet<i64>
}

impl TgWhitelist {
    fn priv_ok(&self, id: i64) -> bool {
        let res = (!self.priv_enabled || self.allowed.contains(&id)) && !self.blocked.contains(&id);
        d!(println!("priv_ok({}) = {}", id, res));
        res
    }

    fn group_ok(&self, id: i64) -> bool {
        let res = (!self.group_enabled || self.allowed.contains(&id)) && !self.blocked.contains(&id);
        d!(println!("group_ok({}) = {}", id, res));
        res
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

    fn save(&self, path: &str) {
        match util::encode(&self, path) {
            Ok(()) => (),
            Err(err) => println!("warn: failed to save whitelist: {}", err)
        };
}
}

struct TgSvc {
    config: TgCfg,
    whitelist: RwLock<TgWhitelist>,
    service: EvalSvc,
    own_id: i64
}

impl TgSvc {
    fn init() -> Result<TgSvc, ()> {
        let config = match util::decode::<TgCfg>("evalbot.tg.toml") {
            Ok(x) => x,
            Err(x) => {
                println!("failed to read evalbot.tg.toml: {:?}", x);
                return Err(());
            }
        };

        let whitelist = match util::decode::<TgWhitelist>(WHITELIST_FILENAME) {
            Ok(x) => x,
            Err(x) => {
                println!("failed to read whitelist: {:?}, using default empty one", x);
                TgWhitelist {
                    priv_enabled: false,
                    group_enabled: false,
                    allowed: HashSet::new(),
                    blocked: HashSet::new(),
                }
            }
        };

        whitelist.save(WHITELIST_FILENAME);

        let me = match tgapi::get_me(&config.bot_id) {
            Ok(x) => x,
            Err(err) => {
                println!("failed to get_me: {}", err);
                return Err(());
            }
        };

        Ok(TgSvc {
            config: config,
            service: EvalSvc::new(match util::decode::<EvalSvcCfg>("evalbot.toml") {
                Ok(x) => x,
                Err(x) => {
                    println!("failed to read evalbot.toml: {:?}", x);
                    return Err(());
                }
            }),
            whitelist: RwLock::new(whitelist),
            own_id: me.id
        })
    }

    fn is_owner(&self, user: &tgapi::recv::User) -> bool {
        if let tgapi::recv::User { username: Some(ref username), .. } = *user {
            self.config.owners.contains(username)
        } else {
            false
        }
    }

    fn is_from_owner(&self, msg: &tgapi::recv::Message) -> bool {
        if let Some(ref user) = msg.from {
            self.is_owner(user)
        } else {
            false
        }
    }

    fn handle(&self, req: &str) -> Result<String, StatusCode> {
        macro_rules! ignore_req {
            () => { return Ok("".to_owned()); }
        }
        let update = match serde_json::from_str::<tgapi::recv::Update>(req) {
            Ok(update) => update,
            Err(err) => {
                println!("failed to parse update: {}\n{}", err, req);
                return Err(StatusCode::BAD_REQUEST);
            }
        };

        d!(println!("decoded into:\n{:?}", update));

        let message_obj = match update.message {
            Some(msg) => msg,
            None => ignore_req!(),
        };

        if message_obj.text.is_some() {
            self.handle_text_message(message_obj)
        } else if message_obj.new_chat_member.is_some() {
            self.handle_join_group(message_obj)
        } else {
            ignore_req!();
        }
    }

    fn handle_join_group(&self, message_obj: tgapi::recv::Message) -> Result<String, StatusCode> {
        d!(println!("handle_join_group"));
        let newmember = message_obj.new_chat_member.unwrap();
        if let Ok(wl) = self.whitelist.read() {
            if newmember.id != self.own_id || wl.group_ok(message_obj.chat.id) {
                ignore_req!();
            }
        }

        if let Some(oid) = self.config.msg_owner_id {
            match tgapi::send_message(&self.config.bot_id, oid,
                format!("Bot was added to group {} not in whitelist", message_obj.chat.id),
                None, None) {
                Ok(()) => (),
                Err(err) => println!("Error calling send_message: {}", err)
            };
        }

        match tgapi::send_message(&self.config.bot_id, message_obj.chat.id,
            format!("This group is not on the whitelist. ID: {}", message_obj.chat.id),
            None, None) {
            Ok(()) => (),
            Err(err) => println!("Error calling send_message: {}", err)
        };

        tgapi::respond_leave_group(message_obj.chat.id).map_err(|err| {
            println!("Error calling respond_leave_group: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }

    fn handle_text_message(&self, message_obj: tgapi::recv::Message) -> Result<String, StatusCode> {
        d!(println!("handle_text_message"));
        let owner = self.is_from_owner(&message_obj);
        let (dollar_cmd, slash_cmd) = {
            let message = message_obj.text.as_ref().unwrap();
            (message.starts_with('$'), message.starts_with('/'))
        };

        if owner && dollar_cmd && message_obj.chat.chat_type == "private" {
            self.handle_owner_command(message_obj)
        } else if slash_cmd {
            self.handle_eval(message_obj)
        } else {
            ignore_req!();
        }
    }

    fn handle_owner_command(&self, message_obj: tgapi::recv::Message) -> Result<String, StatusCode> {
        d!(println!("handle_owner_command"));
        let message = message_obj.text.unwrap();

        let tok = message.trim().split_whitespace().collect::<Vec<_>>();
        if tok.len() < 1 { // this can't happen actually..
            ignore_req!();
        }

        let resp: String = match &tok[0][1..] {
            "privwl" => {
                match self.whitelist.write() {
                    Ok(mut wl) => {
                        wl.priv_enabled = !wl.priv_enabled;
                        wl.save(WHITELIST_FILENAME);
                        format!("Private whitelist enabled: {}", wl.priv_enabled)
                    }
                    Err(err) => format!("Error while acquiring RwLock: {}", err)
                }
            }
            "groupwl" => {
                match self.whitelist.write() {
                    Ok(mut wl) => {
                        wl.group_enabled = !wl.group_enabled;
                        wl.save(WHITELIST_FILENAME);
                        format!("Group whitelist enabled: {}", wl.group_enabled)
                    }
                    Err(err) => format!("Error while acquiring RwLock: {}", err)
                }
            }
            "allow" if tok.len() >= 2 => {
                match (tok[1].parse(), self.whitelist.write()) {
                    (Ok(id), Ok(mut wl)) => {
                        wl.allow(id);
                        wl.save(WHITELIST_FILENAME);
                        format!("Allowed {}", id)
                    }
                    (Err(_), _) => "Invalid ID".to_owned(),
                    (_, Err(err)) => format!("Error while acquiring RwLock: {}", err)
                }
            }
            "unallow" if tok.len() >= 2 => {
                match (tok[1].parse(), self.whitelist.write()) {
                    (Ok(id), Ok(mut wl)) => {
                        wl.unallow(id);
                        wl.save(WHITELIST_FILENAME);
                        format!("Unallowed {}", id)
                    }
                    (Err(_), _) => "Invalid ID".to_owned(),
                    (_, Err(err)) => format!("Error while acquiring RwLock: {}", err)
                }
            }
            "block" if tok.len() >= 2 => {
                match (tok[1].parse(), self.whitelist.write()) {
                    (Ok(id), Ok(mut wl)) => {
                        wl.block(id);
                        wl.save(WHITELIST_FILENAME);
                        format!("Blocked {}", id)
                    }
                    (Err(_), _) => "Invalid ID".to_owned(),
                    (_, Err(err)) => format!("Error while acquiring RwLock: {}", err)
                }
            }
            "unblock" if tok.len() >= 2 => {
                match (tok[1].parse(), self.whitelist.write()) {
                    (Ok(id), Ok(mut wl)) => {
                        wl.unblock(id);
                        wl.save(WHITELIST_FILENAME);
                        format!("Unblocked {}", id)
                    }
                    (Err(_), _) => "Invalid ID".to_owned(),
                    (_, Err(err)) => format!("Error while acquiring RwLock: {}", err)
                }
            }
            "leave" if tok.len() >= 2 => {
                match tok[1].parse() {
                    Ok(id) => {
                        match tgapi::leave_group(&self.config.bot_id, id) {
                            Ok(()) => "Left group".to_owned(),
                            Err(err) => format!("Error: {}", err)
                        }
                    }
                    Err(_) => "Invalid ID".to_owned()
                }
            }
            _ => {
                "No such command or insufficient parameters".to_owned()
            }
        };

        tgapi::respond_send_msg(message_obj.chat.id, resp, None, Some(message_obj.message_id)).map_err(|err| {
            println!("Error calling respond_send_msg: {}", err);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }

    fn handle_eval(&self, message_obj: tgapi::recv::Message) -> Result<String, StatusCode> {
        d!(println!("handle_eval"));
        if let Ok(wl) = self.whitelist.read() {
            if (message_obj.chat.chat_type != "private" && !wl.group_ok(message_obj.chat.id))
                || (message_obj.chat.chat_type == "private" && !wl.priv_ok(message_obj.chat.id)) {
                return tgapi::respond_send_msg(message_obj.chat.id, format!("You or this group is not on the whitelist. Seek help. ID: {}", message_obj.chat.id), None, None).map_err(|err| {
                    println!("Error calling respond_send_msg: {}", err);
                    StatusCode::INTERNAL_SERVER_ERROR
                });
            }
        } else {
            ignore_req!();
        }

        let mut message = message_obj.text.as_ref().unwrap().replace("\r\n", "\n");

        if !message.ends_with('\n') {
            message.push('\n');
        }

        let message = message;

        let first_tok_rest = message[1..].splitn(2, char::is_whitespace).collect::<Vec<_>>();

        let command = match first_tok_rest[0].splitn(2, '@').nth(0) {
            Some(commandtok) => commandtok,
            None => ignore_req!(),
        };

        let lang = self.config.lang_subst.get(command).map_or(command, |s| &s[..]);
        let code = first_tok_rest.get(1).map_or("", |x| *x);

        let code_lines = code.splitn(2, '\n').collect::<Vec<_>>();
        let code = if let (true, Some(skip_first_line)) = (code_lines.get(0).map_or(false, |x| x.trim().is_empty()), code_lines.get(1)) {
            skip_first_line
        } else {
            code
        };

        let timeout = !(first_tok_rest[0].ends_with('#') && self.is_from_owner(&message_obj));

        let (bot_id, chat_id, orig_msg_id, is_priv) =
            (self.config.bot_id.clone(), message_obj.chat.id, message_obj.message_id, message_obj.chat.chat_type == "private");

        self.service.eval(lang,
                          code.to_owned(),
                          timeout,
                          Some(format!("$tg{}", message_obj.chat.id)),
                          Box::new(move |resp| respond(resp, bot_id, chat_id, orig_msg_id, is_priv)));

        Ok(format!(r#"{{"method": "sendChatAction", "chat_id": {}, "action": "typing"}}"#, chat_id))
    }
}

fn respond(resp: Response, bot_id: String, chat_id: i64, orig_msg_id: i64, is_priv: bool) {
    fn html_escape(src: String) -> String {
        src.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }

    let html = match resp {
        Response::Success(output) => {
            if output.is_empty() {
                "OK, no output.".to_owned()
            } else {
                format!("<pre>{}</pre>", html_escape(output))
            }
        }
        Response::Error(output) => {
            format!("<strong>An Evalbot error occured.</strong>\n<pre>{}</pre>", html_escape(output))
        }
        Response::NoSuchLanguage if is_priv => "No such language.".to_owned(),
        Response::NoSuchLanguage => return,
    };

    match tgapi::send_message(&bot_id, chat_id, html, Some("HTML".to_owned()), Some(orig_msg_id)) {
        Err(err) => {
            println!("error while sending response: {}", err);
        }
        Ok(()) => ()
    };
}

fn main() {
    let tgsvc = match TgSvc::init() {
        Ok(svc) => svc,
        Err(()) => {
            println!("TgSvc::init() failed");
            return;
        }
    };

    let server = Server::bind(&([127, 0, 0, 1], 3000).into())
        .serve(|| hyper::service::service_fn(|req: hyper::Request<Body>| {
            match req.method() {
                &Method::POST => futures::future::Either::A(req.into_body().concat2().then(|chunk| {
                    match chunk {
                        Ok(chunk) => match std::str::from_utf8(&chunk) {
                            Ok(req) => {
                                d!(println!("received req:\n{}", req));

                                match tgsvc.handle(req) {
                                    Ok(resp) => hyper::Response::builder()
                                        .header(hyper::header::CONTENT_TYPE, "application/json; charset=utf-8")
                                        .body(Body::from(resp)),
                                    Err(status) => hyper::Response::builder()
                                        .status(status)
                                        .body(Body::empty())
                                }
                            }
                            Err(e) => hyper::Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(Body::empty())
                        }
                        Err(e) => hyper::Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Body::empty())
                    }
                })),
                _ => futures::future::Either::B(futures::future::result(hyper::Response::builder()
                        .status(StatusCode::METHOD_NOT_ALLOWED)
                        .body(Body::empty())))
            }
        }))
        .map_err(|e| println!("Hyper error: {}", e));

    hyper::rt::run(server);
}

mod tgapi;
