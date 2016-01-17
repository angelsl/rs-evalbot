use cfg;
use std::fmt::Debug;

pub trait Lang : Debug + Send + Sync {
    fn eval(&self, code: &str) -> Result<String, String>;
    fn restart(&self) {}
    fn terminate(&self) {}
}

pub fn new(cfg: cfg::LangCfg,
           playpen_args: Vec<String>,
           sandbox_path: String,
           timeout: usize)
           -> Box<Lang> {
    if cfg.persistent {
        Box::new(compiler::CompilerLang::new(cfg, playpen_args, sandbox_path, timeout))
    } else {
        Box::new(persistent::ReplLang::new(cfg, playpen_args, sandbox_path, timeout))
    }
}

fn wrap_code(raw: &str, cfg: &cfg::LangCfg) -> String {
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
