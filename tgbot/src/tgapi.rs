use std::io::{Read, Write};

use rustc_serialize::{Encodable, json};

use hyper::Url;
use hyper::client::request::Request;
use hyper::header::ContentType;

use hyper::method::Method;
use hyper::status::StatusCode;

pub mod recv {
    use rustc_serialize::{Decodable, Decoder};
    use std::fmt::Debug;

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

    #[derive(Clone, Default, PartialEq, Debug)]
    pub struct Chat {
        pub id: i64,
        pub chat_type: String, // type is a keyword, sigh
        pub title: Option<String>
    }

    impl Decodable for Chat {
        fn decode<D: Decoder>(d: &mut D) -> Result<Chat, D::Error> {
            d.read_struct("Chat", 2, |d| {
                let id = d.read_struct_field("id", 0, i64::decode)?;
                let chat_type = d.read_struct_field("type", 1, String::decode)?;
                let title = d.read_struct_field("title", 2, Option::<String>::decode)?;
                Ok(Chat {
                    id: id,
                    chat_type: chat_type,
                    title: title
                })
            })
        }
    }

    #[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
    pub struct Message {
        pub message_id: i64,
        pub from: Option<User>,
        pub date: i64,
        pub chat: Chat,
        pub text: Option<String>,
        pub new_chat_member: Option<User>
    }

    #[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
    pub struct Response<T: Decodable + Default + Clone + PartialEq + Debug> {
        pub ok: bool,
        pub description: Option<String>,
        pub result: Option<T>
    }
}

pub mod send {
    use rustc_serialize::{Encodable, Encoder};
    #[derive(Clone, Default, PartialEq, Debug)]
    pub struct Message {
        pub chat_id: i64,
        pub text: String,
        pub parse_mode: Option<String>,
        pub reply_to_message_id: Option<i64>,
        pub method: Option<String>
    }

    // TODO FIXME maybe serde can handle don't-emit-field-at-all-if-None
    impl Encodable for Message {
        fn encode<S: Encoder>(&self, s: &mut S) -> Result<(), S::Error> {
            let no_fields = 2
                + if self.parse_mode.is_some() { 1 } else { 0 }
                + if self.reply_to_message_id.is_some() { 1 } else { 0 }
                + if self.method.is_some() { 1 } else { 0 };
            s.emit_struct("Message", no_fields, |s| {
                let mut i = 1;
                s.emit_struct_field("chat_id", 0, |s| { s.emit_i64(self.chat_id) })?;
                s.emit_struct_field("text", 1, |s| { s.emit_str(&self.text) })?;
                if let Some(ref x) = self.parse_mode {
                    s.emit_struct_field("parse_mode", { i += 1; i }, |s| { s.emit_str(x) })?;
                }
                if let Some(x) = self.reply_to_message_id {
                    s.emit_struct_field("reply_to_message_id", { i += 1; i }, |s| { s.emit_i64(x) })?;
                }
                if let Some(ref x) = self.method {
                    s.emit_struct_field("method", { i += 1; i }, |s| { s.emit_str(x) })?;
                }
                Ok(())
            })
        }
    }

    #[derive(Clone, Default, PartialEq, Debug)]
    pub struct LeaveGroup {
        pub chat_id: i64,
        pub method: Option<String>
    }

    // TODO FIXME FIXME FIXME
    impl Encodable for LeaveGroup {
        fn encode<S: Encoder>(&self, s: &mut S) -> Result<(), S::Error> {
            s.emit_struct("LeaveGroup", if self.method.is_some() { 2 } else { 1 }, |s| {
                s.emit_struct_field("chat_id", 0, |s| { s.emit_i64(self.chat_id) })?;
                if let Some(ref x) = self.method {
                    s.emit_struct_field("method", 1, |s| { s.emit_str(x) })?;
                }
                Ok(())
            })
        }
    }
}

pub fn respond_leave_group(chat_id: i64) -> Result<String, String> {
    json::encode(&send::LeaveGroup {
        chat_id: chat_id,
        method: Some("leaveChat".to_owned())
    }).map_err(|err| format!("error encoding JSON: {}", err))
}

pub fn respond_send_msg(chat_id: i64, text: String, parse_mode: Option<String>, reply_to: Option<i64>) -> Result<String, String> {
    json::encode(&send::Message {
        chat_id: chat_id,
        text: text,
        parse_mode: parse_mode,
        reply_to_message_id: reply_to,
        method: Some("sendMessage".to_owned())
    }).map_err(|err| format!("error encoding JSON: {}", err))
}

pub fn get_me(bot_id: &str) -> Result<recv::User, String> {
    let json_user = get(bot_id, "getMe")?;
    let resp: recv::Response<recv::User> = json::decode(&json_user).map_err(|err| format!("error decoding JSON: {}", err))?;
    if let (true, Some(user)) = (resp.ok, resp.result) {
        Ok(user)
    } else {
        Err(resp.description.unwrap_or("No error message provided".to_owned()))
    }
}

pub fn leave_group(bot_id: &str, chat_id: i64) -> Result<(), String> {
    let res = post(&send::LeaveGroup { chat_id: chat_id, method: None }, bot_id, "leaveChat")?;
    let resp: recv::Response<()> = json::decode(&res).map_err(|err| format!("error decoding JSON: {}", err))?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.description.unwrap_or("No error message provided".to_owned()))
    }
}

pub fn send_message(bot_id: &str, chat_id: i64, text: String, parse_mode: Option<String>, reply_to: Option<i64>) -> Result<(), String> {
    let msg = send::Message {
        chat_id: chat_id,
        reply_to_message_id: reply_to,
        parse_mode: parse_mode,
        text: text,
        method: None
    };

    let res = post(&msg, bot_id, "sendMessage")?;
    // TODO FIXME repetitive
    let resp: recv::Response<recv::Message> = json::decode(&res).map_err(|err| format!("error decoding JSON: {}", err))?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.description.unwrap_or("No error message provided".to_owned()))
    }
}

pub fn get(bot_id: &str, method: &str) -> Result<String, String> {
    let url = Url::parse(&format!("https://api.telegram.org/bot{}/{}", bot_id, method))
        .map_err(|err| format!("error parsing URL: {}", err))?;
    let req = Request::new(Method::Get, url).map_err(|err| format!("error creating hyper request: {}", err))?;
    let req = req.start().map_err(|err| format!("error starting hyper request: {}", err))?;

    match req.send() {
        Ok(mut resp) => {
            let mut buf = String::new();
            match (resp.read_to_string(&mut buf), resp.status) {
                (Ok(_), StatusCode::Ok) => Ok(buf),
                (Ok(_), _) => Err(format!("Telegram returned status {}:\n{}", resp.status, buf)),
                (Err(err), _) => Err(format!("Telegram returned status {} and error reading response:\n{}", resp.status, err))
            }
        }
        Err(err) => Err(format!("error GETing from Telegram: {}", err))
    }
}

pub fn post<T: Encodable>(msg: &T, bot_id: &str, method: &str) -> Result<String, String> {
    let url = Url::parse(&format!("https://api.telegram.org/bot{}/{}", bot_id, method))
        .map_err(|err| format!("error parsing URL: {}", err))?;
    let json = json::encode(msg).map_err(|err| format!("error encoding JSON: {}", err))?;
    let mut req = Request::new(Method::Post, url).map_err(|err| format!("error creating hyper request: {}", err))?;
    req.headers_mut().set(ContentType(mime!(Application/Json; Charset=Utf8)));
    let mut req = req.start().map_err(|err| format!("error starting hyper request: {}", err))?;

    req.write_all(json.as_bytes()).map_err(|err| format!("error writing JSON to server: {}", err))?;

    match req.send() {
        Ok(mut resp) => {
            let mut buf = String::new();
            match (resp.read_to_string(&mut buf), resp.status) {
                (Ok(_), StatusCode::Ok) => Ok(buf),
                (Ok(_), _) => Err(format!("Telegram returned status {}:\n{}", resp.status, buf)),
                (Err(err), _) => Err(format!("Telegram returned status {} and error reading response:\n{}", resp.status, err))
            }
        }
        Err(err) => Err(format!("error POSTing to Telegram: {}", err))
    }
}
