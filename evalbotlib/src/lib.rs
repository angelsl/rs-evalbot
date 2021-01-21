use std::collections::HashMap;
use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use log::debug;
use serde::{Deserialize, Serialize};

mod eval;
pub mod util;

#[derive(Clone, Serialize, Deserialize, Default, PartialEq, Debug)]
struct EvalServiceCfg {
    timeout: usize,
    languages: HashMap<String, LanguageCfg>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
struct LanguageCfg {
    code_before: Option<String>,
    code_after: Option<String>,
    timeout: Option<usize>,
    #[serde(flatten)]
    backend: BackendCfg,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(untagged)]
enum BackendCfg {
    Exec(ExecBackend),
    Network(NetworkBackend),
    UnixSocket(UnixSocketBackend),
}

#[derive(Clone, Debug)]
pub struct EvalService {
    timeout: usize,
    languages: HashMap<String, Arc<Language>>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Language {
    name: String,
    code_before: Option<String>,
    code_after: Option<String>,
    timeout: Option<usize>,
    backend: Backend,
}

#[derive(Clone, PartialEq, Debug)]
enum Backend {
    Exec(Arc<ExecBackend>),
    Network(Arc<NetworkBackend>),
    UnixSocket(Arc<UnixSocketBackend>),
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct ExecBackend {
    cmdline: Vec<String>,
    timeout_prefix: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct NetworkBackend {
    network_addr: String,
    timeout_cmdline: Option<Vec<String>>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct UnixSocketBackend {
    socket_addr: String,
    timeout_cmdline: Option<Vec<String>>,
}

impl Language {
    fn from(name: String, default_timeout: usize, cfg: LanguageCfg) -> Self {
        Language {
            name,
            code_before: cfg.code_before,
            code_after: cfg.code_after,
            timeout: cfg.timeout.or_else(|| Some(default_timeout)),
            backend: match cfg.backend {
                BackendCfg::Exec(x) => Backend::Exec(Arc::new(x)),
                BackendCfg::Network(x) => Backend::Network(Arc::new(x)),
                BackendCfg::UnixSocket(x) => Backend::UnixSocket(Arc::new(x)),
            },
        }
    }
}

impl EvalService {
    fn fixup(cfg: EvalServiceCfg) -> Self {
        debug!("Loaded config: {:#?}", cfg);
        let mut new = EvalService {
            timeout: cfg.timeout,
            languages: HashMap::new(),
        };
        let timeout = cfg.timeout;
        for (name, lang) in cfg.languages.into_iter() {
            new.languages
                .insert(name.clone(), Arc::new(Language::from(name, timeout, lang)));
        }
        new
    }

    pub async fn from_toml_file<P>(path: P) -> Result<Self, String>
    where
        P: AsRef<Path> + Send + Display + 'static,
    {
        Ok(EvalService::fixup(util::decode(path).await?))
    }

    pub fn from_toml(toml: &str) -> Result<Self, String> {
        toml::from_str(toml)
            .map(EvalService::fixup)
            .map_err(|x| format!("could not parse TOML: {:?}", x))
    }

    pub fn langs(&self) -> impl Iterator<Item = (&str, &Arc<Language>)> {
        self.languages.iter().map(|(n, l)| (n.as_str(), l))
    }

    pub fn get(&self, lang: &str) -> Option<&Arc<Language>> {
        self.languages.get(lang)
    }
}

static EMPTY_U8: [u8; 0] = [];

impl Language {
    pub async fn eval<T, U>(
        &self,
        code: T,
        timeout: Option<usize>,
        context: Option<U>,
    ) -> Result<String, String>
    where
        T: AsRef<str>,
        U: AsRef<str>,
    {
        debug!("evaluating {}: \"{}\"", self.name, code.as_ref());
        let timeout = match timeout {
            Some(0) => None,
            Some(n) => Some(n),
            None => self.timeout,
        };
        match self.backend {
            Backend::Exec(ref lang) => {
                eval::exec(lang.clone(), timeout, self.wrap_code(code.as_ref())).await
            }
            Backend::UnixSocket(ref lang) => {
                eval::unix(
                    lang.clone(),
                    timeout,
                    context.map(|x| x.as_ref().to_owned()), // FIXME copy :(
                    self.wrap_code(code.as_ref()),
                )
                .await
            }
            _ => Ok("Unimplemented".to_owned()),
        }
    }

    fn wrap_code(&self, raw: &str) -> String {
        let mut code = String::with_capacity(raw.len());

        if let Some(ref prefix) = self.code_before {
            code.push_str(prefix);
        }

        code.push_str(raw);

        if let Some(ref postfix) = self.code_after {
            code.push_str(postfix);
        }

        code
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_decode() {
        let toml = r#"
timeout = 20

[languages.rs]
cmdline = ["rustc", "-O"]

[languages.'rs!']
timeout = 0
cmdline = ["rustc", "-O"]
"#;
        println!("{:#?}", super::EvalService::from_toml(toml).unwrap());
    }
}
