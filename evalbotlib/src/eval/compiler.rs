use {eval, playpen, std};

#[derive(Clone)]
pub struct CompilerLang {
    cfg: ::LangCfg,
    playpen_args: Vec<String>,
    sandbox_path: String,
    timeout: usize
}

impl CompilerLang {
    pub fn new(cfg: ::LangCfg, playpen_args: Vec<String>, sandbox_path: String, timeout: usize) -> Self {
        CompilerLang {
            cfg: cfg,
            playpen_args: playpen_args,
            sandbox_path: sandbox_path,
            timeout: timeout
        }
    }
}

unsafe impl Send for CompilerLang {}
unsafe impl Sync for CompilerLang {}

impl eval::Lang for CompilerLang {
    fn eval(&self, code: &str, with_timeout: bool, _: Option<&str>) -> Result<String, String> {
        let code = eval::wrap_code(code, &self.cfg);
        playpen::exec_wait(&self.sandbox_path,
                           &self.cfg.binary_path,
                           &self.cfg.syscalls_path,
                           &self.playpen_args,
                           &self.cfg.binary_args,
                           &code,
                           if with_timeout { Some(self.timeout) } else { None })
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
