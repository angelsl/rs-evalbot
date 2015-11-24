use std::io::prelude::*;
use std::fs::File;
use rustc_serialize::Decodable;
use toml;

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
pub struct EvalbotCfg {
    pub command_prefix: String,
    pub chan_output_prefix: String,
    pub max_channel_lines: usize,
    pub max_channel_line_len: usize,
    pub max_priv_lines: usize,
    pub sandbox_dir: String,
    pub playpen_timeout: usize,
    pub playpen_args: Vec<String>,
    pub eval_threads: usize,
    pub owners: Vec<String>,
    pub languages: Vec<LangCfg>
}

#[derive(Clone, RustcDecodable, Default, PartialEq, Debug)]
pub struct LangCfg {
    pub syscalls_path: String,
    pub binary_path: String,
    pub binary_args: Vec<String>,
    pub persistent: bool,
    pub long_name: String,
    pub short_name: String,
    pub code_before: Option<String>,
    pub code_after: Option<String>
}

unsafe impl Send for EvalbotCfg {}
unsafe impl Send for LangCfg {}

pub fn read<T>(name: &str) -> Result<T, String>
    where T: Decodable {
    let mut f = try!(File::open(name).map_err(|x| format!("could not open {}: {}", name, x)));
    let mut s = String::new();

    try!(f.read_to_string(&mut s)
          .map_err(|x| format!("could not read {}: {}", name, x)));

    let value = try!(s.parse::<toml::Value>()
                      .map_err(|x| format!("could not parse {}: {:?}", name, x)));

    T::decode(&mut toml::Decoder::new(value))
        .map_err(|x| format!("could not decode {}: {}", name, x))
}
