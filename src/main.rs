extern crate irc;
extern crate toml;
extern crate rustc_serialize;

use irc::client::prelude::*;

#[derive(Clone, RustcDecodable, RustcEncodable, Default, PartialEq, Debug)]
struct EvalbotCfg {
    pub max_channel_lines: u32,
    pub max_channel_line_len: u32,
    pub playpen_timeout: u32,
    pub irc_config: Config
}

fn read_cfg() -> Result<EvalbotCfg, String> {
    use std::io::prelude::*;
    use std::fs::File;
    use rustc_serialize::Decodable;

    let mut f = match File::open("evalbot.toml") {
        Ok(x) => x,
        Err(x) => return Err(format!("could not open evalbot.toml: {}", x))
    };
    let mut s = String::new();
    match f.read_to_string(&mut s) {
        Err(x) => return Err(format!("could not read evalbot.toml: {}", x)),
        _ => ()
    };

    let value = match s.parse::<toml::Value>() {
        Ok(x) => x,
        Err(x) => return Err(format!("could not parse evalbot.toml: {:?}", x))
    };

    let res = match EvalbotCfg::decode(&mut toml::Decoder::new(value)) {
        Ok(x) => x,
        Err(x) => return Err(format!("could not decde evalbot.toml: {}", x))
    };

    Ok(res)
}

fn parse_msg(msg: &Message) -> Option<(bool, String, String)> {
    if let Ok(Command::PRIVMSG(tgt, msg)) = msg.into() {
        let chn = tgt.starts_with('#');
        if !chn && msg.contains('\x01') { return None; }
        let tok: Vec<&str> = msg.trim().splitn(2, '>').collect();
        if tok.len() < 2 && chn { None }
        else if chn {
            match tok[0] {
                "rust" => Some((false, tgt, tok[1].to_owned())),
                "rustraw" => Some((true, tgt, tok[1].to_owned())),
                _ => None
            }
        } else {
            match tok[0] {
                "raw" => Some((true, tgt, tok[1].to_owned())),
                _ => Some((false, tgt, msg.to_owned()))
            }
        }
    } else {
        None
    }
}

fn main() {
    let config = match read_cfg() {
        Ok(x) => x,
        Err(x) => panic!("could not read config; {}", x)
    };
    println!("read config: {:?}", config);

    let conn = IrcServer::from_config(config.irc_config).unwrap();
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

            let src = msg.get_source_nickname().unwrap_or("");
            if let Some((raw, tgt, code)) = parse_msg(&msg) {
                println!("{} @ {} (raw: {}): {}", src, tgt, raw, code);
                // TODO: evaluate code
            }
        }
    }
}
