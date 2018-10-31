use estring;
use futures::Future;
use futures::Stream;
use hyper::{Body, Chunk, Client, Uri, Request, StatusCode};
use hyper::client::HttpConnector;
use hyper_tls::HttpsConnector;
use serde::Serialize;

pub mod recv {
    use std::fmt::Debug;

    #[derive(Clone, Deserialize, Default, PartialEq, Debug)]
    pub struct Update {
        pub update_id: i64,
        pub message: Option<Message> // the rest of the update types we don't care about
    }

    #[derive(Clone, Deserialize, Default, PartialEq, Debug)]
    pub struct User {
        pub id: i64,
        pub username: Option<String>
    }

    #[derive(Clone, Deserialize, Default, PartialEq, Debug)]
    pub struct Chat {
        pub id: i64,
        #[serde(rename = "type")]
        pub chat_type: String, // type is a keyword, sigh
        pub title: Option<String>
    }

    #[derive(Clone, Deserialize, Default, PartialEq, Debug)]
    pub struct Message {
        pub message_id: i64,
        pub from: Option<User>,
        pub date: i64,
        pub chat: Chat,
        pub text: Option<String>,
        pub new_chat_member: Option<User>
    }

    #[derive(Clone, Deserialize, Default, PartialEq, Debug)]
    pub struct Response<T: Default + Clone + PartialEq + Debug> {
        pub ok: bool,
        pub description: Option<String>,
        pub result: Option<T>
    }
}

pub mod send {
    #[derive(Clone, Serialize, Default, PartialEq, Debug)]
    pub struct Message {
        pub chat_id: i64,
        pub text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub parse_mode: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub reply_to_message_id: Option<i64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub method: Option<String>
    }

    #[derive(Clone, Serialize, Default, PartialEq, Debug)]
    pub struct LeaveGroup {
        pub chat_id: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub method: Option<String>
    }
}

pub fn respond_leave_group(chat_id: i64) -> Result<String, String> {
    serde_json::to_string(&send::LeaveGroup {
        chat_id,
        method: Some("leaveChat".to_owned())
    }).map_err(|err| format!("error encoding JSON: {}", err))
}

pub fn respond_send_msg(chat_id: i64, text: String, parse_mode: Option<String>, reply_to: Option<i64>) -> Result<String, String> {
    serde_json::to_string(&send::Message {
        chat_id,
        text,
        parse_mode,
        reply_to_message_id: reply_to,
        method: Some("sendMessage".to_owned())
    }).map_err(|err| format!("error encoding JSON: {}", err))
}

pub fn get_me(bot_id: &str) -> Result<recv::User, String> {
    let json_user = get(bot_id, "getMe")?;
    let resp: recv::Response<recv::User> = serde_json::from_str(&json_user).map_err(|err| format!("error decoding JSON: {}", err))?;
    if let (true, Some(user)) = (resp.ok, resp.result) {
        Ok(user)
    } else {
        Err(resp.description.unwrap_or_else(|| "No error message provided".to_owned()))
    }
}

pub fn leave_group(bot_id: &str, chat_id: i64) -> Result<(), String> {
    let res = post(&send::LeaveGroup { chat_id, method: None }, bot_id, "leaveChat")?;
    let resp: recv::Response<()> = serde_json::from_str(&res).map_err(|err| format!("error decoding JSON: {}", err))?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.description.unwrap_or_else(|| "No error message provided".to_owned()))
    }
}

pub fn send_message(bot_id: &str, chat_id: i64, text: String, parse_mode: Option<String>, reply_to: Option<i64>) -> Result<(), String> {
    let msg = send::Message {
        chat_id,
        reply_to_message_id: reply_to,
        parse_mode,
        text,
        method: None
    };

    let res = post(&msg, bot_id, "sendMessage")?;
    // TODO FIXME repetitive
    let resp: recv::Response<recv::Message> = serde_json::from_str(&res).map_err(|err| format!("error decoding JSON: {}", err))?;
    if resp.ok {
        Ok(())
    } else {
        Err(resp.description.unwrap_or_else(|| "No error message provided".to_owned()))
    }
}

thread_local! {
    static http_client: Client<HttpsConnector<HttpConnector>> = Client::builder().build::<_, Body>(
        HttpsConnector::new(1).unwrap());
}

pub fn get(bot_id: &str, method: &str) -> Result<estring::String<Chunk>, String> {
    let url = format!("https://api.telegram.org/bot{}/{}", bot_id, method).parse::<Uri>()
        .map_err(|err| format!("error parsing URL: {}", err))?;
    let req = Request::get(url).body(Body::from(""))
        .map_err(|err| format!("error creating hyper request: {}", err))?;
    // FIXME need to re-write totally to use Tokio properly, instead of just wait()
    http_client.with(|x| x.request(req)).wait()
        .map_err(|e| format!("error GETing from Telegram: {}", e))
        .and_then(|resp| {
            let status = resp.status();
            let resp: Result<estring::String<Chunk>, String> = resp.into_body().concat2().wait()
                .map_err(|e| format!("error receiving response: {}", e))
                .and_then(|c| estring::TryFrom::try_from(c)
                    .map_err(|e| format!("error decoding response: {}", e)));
            match (status, resp) {
                (StatusCode::OK, Ok(resp)) => Ok(resp),
                (_, Ok(resp)) => Err(format!("Telegram returned status {}:\n{}", status, &resp[..])),
                (_, Err(e)) =>  Err(format!("Telegram returned status {} and error resding response:\n{}", status, e))
            }
        })
}

pub fn post<T: Serialize>(msg: &T, bot_id: &str, method: &str) -> Result<estring::String<Chunk>, String> {
    let url = format!("https://api.telegram.org/bot{}/{}", bot_id, method).parse::<Uri>()
        .map_err(|err| format!("error parsing URL: {}", err))?;
    let json = serde_json::to_string(msg).map_err(|err| format!("error encoding JSON: {}", err))?;
    let req = Request::post(url)
        .header("Content-Type", "application/json; charset=utf-8")
        .body(json.into())
        .map_err(|err| format!("error creating hyper request: {}", err))?;
    http_client.with(|x| x.request(req)).wait()
        .map_err(|e| format!("error POSTing to Telegram: {}", e))
        .and_then(|resp| {
            let status = resp.status();
            let resp: Result<estring::String<Chunk>, String> = resp.into_body().concat2().wait()
                .map_err(|e| format!("error receiving response: {}", e))
                .and_then(|c| estring::TryFrom::try_from(c)
                    .map_err(|e| format!("error decoding response: {}", e)));
            match (status, resp) {
                (StatusCode::OK, Ok(resp)) => Ok(resp),
                (_, Ok(resp)) => Err(format!("Telegram returned status {}:\n{}", status, &resp[..])),
                (_, Err(e)) =>  Err(format!("Telegram returned status {} and error resding response:\n{}", status, e))
            }
        })
}

