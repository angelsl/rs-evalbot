use irc::client::data::config::Config;
use std::io::prelude::*;
use std::fs::File;
use rustc_serialize::Decodable;
use toml;

#[derive(Clone, RustcDecodable, RustcEncodable, Default, PartialEq, Debug)]
pub struct EvalbotCfg {
    pub chan_output_prefix: String,
    pub sandbox_dir: String,
    pub max_channel_lines: usize,
    pub max_channel_line_len: usize,
    pub max_priv_lines: usize,
    pub playpen_timeout: usize,
    pub eval_threads: usize,
    pub irc_config: Config
}

unsafe impl Send for EvalbotCfg {}

pub fn read(name: &str) -> Result<EvalbotCfg, String> {
    let mut f = try!(File::open(name)
                     .map_err(|x| format!("could not open {}: {}", name, x)));
    let mut s = String::new();

    try!(f.read_to_string(&mut s)
         .map_err(|x| format!("could not read {}: {}", name, x)));

    let value = try!(s.parse::<toml::Value>()
                     .map_err(|x| format!("could not parse {}: {:?}", name, x)));

    EvalbotCfg::decode(&mut toml::Decoder::new(value))
         .map_err(|x| format!("could not decode {}: {}", name, x))
}
