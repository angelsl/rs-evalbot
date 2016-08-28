use std::fmt::Debug;
use std::sync::Arc;

pub trait Lang: Debug + Send + Sync {
    fn eval(&self, code: &str, timeout: Option<usize>, context_key: Option<&str>) -> Result<String, String>;
}

pub fn new(cfg: &::LangCfg) -> Arc<Lang> {
    if {
        (if cfg.binary_path.is_some() { 1 } else { 0 })
        + (if cfg.network_address.is_some() { 1 } else { 0 })
        + (if cfg.socket_address.is_some() { 1 } else { 0 })
    } != 1 {
        panic!("LangCfg with more or less than one of binary_path, network_address, socket_address specified");
    }

    if cfg.binary_path.is_some() {
        Arc::new(exec::ExecLang::new(cfg.binary_path.as_ref().unwrap().to_owned(), cfg.binary_args.as_ref().unwrap().clone(), cfg.binary_timeout_arg.clone()))
    } else if cfg.network_address.is_some() {
        Arc::new(remote::network::NetworkLang::new(cfg.network_address.as_ref().unwrap().to_owned()))
    } else if cfg.socket_address.is_some() {
        Arc::new(remote::unixsocket::UnixSocketLang::new(cfg.socket_address.as_ref().unwrap().to_owned()))
    } else {
        panic!("No valid run configuration found")
    }
}

mod exec;
mod remote;
