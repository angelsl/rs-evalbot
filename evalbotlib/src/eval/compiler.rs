use {eval, playpen, std};

#[derive(Clone)]
pub struct CompilerLang {
    cfg: ::LangCfg
}

impl CompilerLang {
    pub fn new(cfg: ::LangCfg) -> Self {
        CompilerLang {
            cfg: cfg
        }
    }
}

unsafe impl Send for CompilerLang {}
unsafe impl Sync for CompilerLang {}

impl eval::Lang for CompilerLang {
    fn eval(&self, code: &str, with_timeout: bool, _: Option<&str>) -> Result<String, String> {
        let code = eval::wrap_code(code, &self.cfg);
        playpen::exec_wait(&self.cfg.binary_path,
                           &self.cfg.args(with_timeout),
                           &code)
    }

    fn is_persistent(&self) -> bool {
        false
    }
}

impl std::fmt::Debug for CompilerLang {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("CompilerLang").field("cfg", &self.cfg).finish()
    }
}
