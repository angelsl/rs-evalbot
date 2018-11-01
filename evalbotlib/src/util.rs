use tokio::prelude::*;
use tokio::io::{read_to_end, write_all};
use tokio::fs::File;

use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;
use std::fmt::Display;

pub fn encode<'a, 'b, T, P>(obj: &'a T, name: P) -> impl Future<Item = (), Error = String> + 'b
    where
        P: AsRef<Path> + Send + Display + 'static,
        T: Serialize {
    toml::to_string(obj).map_err(|e| format!("toml encode failed: {}", e)).into_future()
        .and_then(|s| File::create(name)
            .map(|f| (f, s))
            .map_err(|x| format!("could not open file: {}", x)))
        .and_then(|(f, toml)| write_all(f, toml.into_bytes())
            .map_err(|x| format!("could not write to file: {}", x)))
        .map(|_| ())
}

pub fn decode<T, P>(name: P) -> impl Future<Item = T, Error = String>
    where
        P: AsRef<Path> + Send + Display + 'static,
        T: DeserializeOwned {
    File::open(name).map_err(|x| format!("could not open file: {}", x))
        .and_then(|f| read_to_end(f, Vec::new())
            .map_err(|x| format!("could not read file: {}", x)))
        .and_then(|(_, buf)| toml::from_str(&String::from_utf8_lossy(&buf[..]))
            .map_err(|x| format!("could not parse file: {:?}", x)))
}
