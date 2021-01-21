use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Display;
use std::path::Path;

use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn encode<'a, T, P>(obj: &'a T, name: P) -> Result<(), String>
where
    P: AsRef<Path> + Send + Display + 'static,
    T: Serialize,
{
    let toml_string = toml::to_string(obj).map_err(|e| format!("toml encode failed: {}", e))?;
    let mut file = File::create(name)
        .await
        .map_err(|x| format!("could not open file: {}", x))?;
    file.write_all(&toml_string.into_bytes())
        .await
        .map_err(|x| format!("could not write to file: {}", x))?;
    Ok(())
}

pub async fn decode<T, P>(name: P) -> Result<T, String>
where
    P: AsRef<Path> + Send + Display + 'static,
    T: DeserializeOwned,
{
    let mut file = File::open(name)
        .await
        .map_err(|x| format!("could not open file: {}", x))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .await
        .map_err(|x| format!("could not read file: {}", x))?;
    toml::from_str(&String::from_utf8_lossy(&buf[..]))
        .map_err(|x| format!("could not parse file: {:?}", x))
}
