use std::sync::Arc;
use std::fmt::Debug;

pub trait Lang: Debug + Send + Sync {
    fn eval(&self, code: &str, with_timeout: bool, context_key: Option<&str>) -> Result<String, String>;
    fn restart(&self) {}
    fn is_persistent(&self) -> bool;
}

pub fn new(cfg: ::LangCfg, playpen_args: Vec<String>, sandbox_path: String, timeout: usize) -> Arc<Lang> {
    if cfg.persistent {
        Arc::new(persistent::ReplLang::new(cfg, playpen_args, sandbox_path, timeout))
    } else {
        Arc::new(compiler::CompilerLang::new(cfg, playpen_args, sandbox_path, timeout))
    }
}

fn wrap_code(raw: &str, cfg: &::LangCfg) -> String {
    let mut code = String::with_capacity(raw.len());

    if let Some(ref prefix) = cfg.code_before {
        code.push_str(prefix);
    }

    code.push_str(raw);

    if let Some(ref postfix) = cfg.code_after {
        code.push_str(postfix);
    }

    code
}

mod compiler;
mod persistent;
