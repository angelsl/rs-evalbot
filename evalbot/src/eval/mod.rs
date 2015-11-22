use cfg;

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

pub mod script {
    use {cfg, eval, playpen};

    pub fn eval(raw: &str, cfg: &cfg::LangCfg, sandbox: &str, timeout: usize) -> Result<String, String> {
        let code = eval::wrap_code(raw, cfg);
        playpen::exec_wait(&sandbox, &cfg.binary_path, &cfg.syscalls_path,
                           &cfg.binary_args.iter().map(|s| &**s).collect::<Vec<&str>>()[..],
                           &code,
                           timeout)
    }
}

pub mod persistent;
