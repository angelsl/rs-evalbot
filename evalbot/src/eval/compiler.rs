use {cfg, eval, playpen, std};

#[derive(Clone)]
pub struct CompilerLang {
    cfg: cfg::LangCfg,
    playpen_args: Vec<String>,
    sandbox_path: String,
    timeout: usize
}

impl CompilerLang {
    pub fn new(cfg: cfg::LangCfg,
               playpen_args: Vec<String>,
               sandbox_path: String,
               timeout: usize)
               -> Self {
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
    fn eval(&self, code: &str) -> Result<String, String> {
        let code = eval::wrap_code(code, &self.cfg);
        playpen::exec_wait(&self.sandbox_path,
                           &self.cfg.binary_path,
                           &self.cfg.syscalls_path,
                           &self.playpen_args,
                           &self.cfg.binary_args,
                           &code,
                           self.timeout)
    }
}

impl std::fmt::Debug for CompilerLang {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "CompilerLang {{ cfg: {:?} }}", self.cfg)
    }
}
