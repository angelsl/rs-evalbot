#![feature(plugin, question_mark)]
#![plugin(clippy)]

extern crate evalbotlib as backend;
extern crate hyper;
#[macro_use]
extern crate mime;
extern crate rustc_serialize;

use backend::{EvalSvc, EvalSvcCfg, Response, util};
use hyper::Url;
use hyper::client::request::Request;
use hyper::header::ContentType;

use hyper::method::Method;
use hyper::status::StatusCode;

use rustc_serialize::json;
use std::collections::HashMap;

use std::io::{Read, Write};

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
struct TgCfg {
    owners: Vec<String>,
    bot_id: String,
    lang_subst: HashMap<String, String>
}

pub struct TgSvc {
    config: TgCfg,
    service: EvalSvc,
    post_url: Url
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

        let post_url = Url::parse(&format!("https://api.telegram.org/bot{}/sendMessage", config.bot_id)).unwrap();

        Ok(TgSvc {
            config: config,
            post_url: post_url,
            service: EvalSvc::new(match util::decode::<EvalSvcCfg>("evalbot.toml") {
                Ok(x) => x,
                Err(x) => {
                    println!("failed to read evalbot.toml: {:?}", x);
                    return Err(());
                }
            })
        })
    }

    fn handle(&self, req: &str) -> Result<String, ()> {
        macro_rules! ignore_req {
            () => { return Ok("".to_owned()); }
        }
        let update = match json::decode::<tgapi::Update>(req) {
            Ok(update) => update,
            Err(err) => {
                println!("failed to parse update: {}", err);
                return Err(());
            }
        };

        let message_obj = match update.message {
            Some(msg) => msg,
            None => ignore_req!(),
        };

        let message = match message_obj.text {
            Some(text) => text,
            None => ignore_req!(),
        };

        if !message.starts_with('/') {
            ignore_req!();
        }

        let first_tok_rest = message[1..].splitn(2, char::is_whitespace).collect::<Vec<_>>();

        if first_tok_rest.len() != 2 {
            ignore_req!();
        }

        let lang = match first_tok_rest[0].splitn(2, '@').nth(0) {
            Some(langtok) => langtok,
            None => ignore_req!(),
        };

        let lang = self.config.lang_subst.get(lang).map_or(lang, |s| &s[..]);
        let code = first_tok_rest[1].trim();

        let timeout = !(first_tok_rest[0].ends_with('#') &&
                        if let Some(tgapi::User { username: Some(username), .. }) = message_obj.from {
            self.config.owners.iter().any(|x| *x == username)
        } else {
            false
        });

        let (post_url, chat_id, orig_msg_id, is_priv) =
            (self.post_url.clone(), message_obj.chat.id, message_obj.message_id, message_obj.chat.first_name.is_some());

        self.service.eval(lang,
                          code.to_owned(),
                          timeout,
                          Some(format!("$tg{}", message_obj.chat.id)),
                          Box::new(move |resp| respond(resp, post_url, chat_id, orig_msg_id, is_priv)));

        Ok(format!(r#"{{"method": "sendChatAction", "chat_id": {}, "action": "typing"}}"#, chat_id))
    }
}

fn respond(resp: Response, post_url: Url, chat_id: i64, orig_msg_id: i64, is_priv: bool) {
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

    let msg = tgapi::SendMessage {
        chat_id: chat_id,
        reply_to_message_id: orig_msg_id,
        parse_mode: "HTML".to_owned(),
        text: html
    };

    let json = match json::encode(&msg) {
        Ok(json) => json,
        Err(err) => {
            println!("error encoding JSON: {}", err);
            return;
        }
    };

    let mut req = match Request::new(Method::Post, post_url) {
        Ok(req) => req,
        Err(err) => {
            println!("error creating hyper request: {}", err);
            return;
        }
    };

    req.headers_mut().set(ContentType(mime!(Application/Json; Charset=Utf8)));

    let mut req = match req.start() {
        Ok(req) => req,
        Err(err) => {
            println!("error starting hyper request: {}", err);
            return;
        }
    };

    match req.write_all(json.as_bytes()) {
        Ok(req) => req,
        Err(err) => {
            println!("error sending JSON via hyper request: {}", err);
            return;
        }
    };

    match req.send() {
        Ok(mut resp) => {
            if resp.status != StatusCode::Ok {
                let mut buf = String::new();
                match resp.read_to_string(&mut buf) {
                    Ok(_) => println!("Telegram returned status {}:\n{}", resp.status, buf),
                    Err(err) => {
                        println!("Telegram returned status {} and error reading response:\n{}", resp.status, err)
                    }
                };
            }
        }
        Err(err) => println!("error POSTing to Telegram: {}", err),
    };
}

fn main() {
    use hyper::server as hsv;

    let tgsvc = match TgSvc::init() {
        Ok(svc) => svc,
        Err(()) => {
            println!("TgSvc::init() failed");
            return;
        }
    };

    let server = match hsv::Server::http("127.0.0.101:18117") {
        Ok(svr) => svr,
        Err(err) => {
            println!("Server::http failed: {}", err);
            return;
        }
    };

    match server.handle(move |mut req: hsv::Request, mut res: hsv::Response| {
        match req.method {
            Method::Post => {
                let mut buf = String::new();
                match req.read_to_string(&mut buf) {
                    Ok(_) => (),
                    Err(err) => {
                        println!("Request::read_to_end failed: {}", err);
                        *res.status_mut() = StatusCode::InternalServerError;
                    }
                };

                let trim = buf.trim();

                if trim.is_empty() {
                    *res.status_mut() = StatusCode::BadRequest;
                    return;
                }

                let resp = match tgsvc.handle(trim) {
                    Ok(resp) => resp,
                    Err(()) => {
                        *res.status_mut() = StatusCode::InternalServerError;
                        return;
                    }
                };

                res.headers_mut().set(ContentType(mime!(Application/Json; Charset=Utf8)));

                match res.send(resp.as_bytes()) {
                    Ok(_) => (),
                    Err(err) => {
                        println!("Response::send failed: {}", err);
                        return;
                    }
                };
            }
            _ => *res.status_mut() = StatusCode::MethodNotAllowed,
        };
    }) {
        Ok(_) => (),
        Err(err) => {
            println!("Server::handle failed: {}", err);
            return;
        }
    };
}

mod tgapi {
    #[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
    pub struct Update {
        pub update_id: i64,
        pub message: Option<Message> // the rest of the update types we don't care about
    }

    #[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
    pub struct User {
        pub id: i64,
        pub username: Option<String>
    }

    #[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
    pub struct Chat {
        pub id: i64,
        // pub type: String // type is a keyword, sigh
        pub first_name: Option<String> // so use this instead
    }

    #[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
    pub struct Message {
        pub message_id: i64,
        pub from: Option<User>,
        pub date: i64,
        pub chat: Chat,
        pub text: Option<String>
    }

    #[derive(Clone, RustcEncodable, Default, PartialEq, Debug)]
    pub struct SendMessage {
        pub chat_id: i64,
        pub text: String,
        pub parse_mode: String,
        pub reply_to_message_id: i64
    }
}
