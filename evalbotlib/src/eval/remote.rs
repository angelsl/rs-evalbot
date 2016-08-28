extern crate byteorder;

use self::byteorder::{NativeEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Read, Write};

macro_rules! t {
    ($x:expr) => {
        $x.map_err(|err| format!("evalbot error: {}: {}", line!(), err))?
    }
}

pub mod network {
    use eval;
    use eval::remote::communicate;
    use std::time::Duration;
    use std::net::TcpStream;

    #[derive(Clone, Debug)]
    pub struct NetworkLang {
        address: String
    }

    unsafe impl Send for NetworkLang {}
    unsafe impl Sync for NetworkLang {}

    impl NetworkLang {
        pub fn new(address: String) -> Self {
            NetworkLang { address: address }
        }
    }

    impl eval::Lang for NetworkLang {
        fn eval(&self, code: &str, timeout: Option<usize>, context_key: Option<&str>) -> Result<String, String> {
            let mut stream = t!(TcpStream::connect(&self.address as &str));
            let stream_timeout = timeout.map(|t| Duration::from_secs(t as u64 + 5));
            t!(stream.set_read_timeout(stream_timeout));
            t!(stream.set_write_timeout(stream_timeout));
            communicate(&mut stream, code, timeout, context_key)
        }
    }
}

#[cfg(feature = "unixsocket")]
pub mod unixsocket {
    use eval;
    use eval::remote::communicate;
    use std::time::Duration;
    use std::os::unix::net::UnixStream;

    #[derive(Clone, Debug)]
    pub struct UnixSocketLang {
        address: String
    }

    unsafe impl Send for UnixSocketLang {}
    unsafe impl Sync for UnixSocketLang {}

    impl UnixSocketLang {
        pub fn new(address: String) -> Self {
            UnixSocketLang { address: address }
        }
    }

    impl eval::Lang for UnixSocketLang {
        fn eval(&self, code: &str, timeout: Option<usize>, context_key: Option<&str>) -> Result<String, String> {
            let mut stream = t!(UnixStream::connect(&self.address as &str));
            let stream_timeout = timeout.map(|t| Duration::from_secs(t as u64 + 5));
            t!(stream.set_read_timeout(stream_timeout));
            t!(stream.set_write_timeout(stream_timeout));
            communicate(&mut stream, code, timeout, context_key)
        }
    }
}

fn communicate<T: Read + Write>(socket: &mut T,
    code: &str,
    timeout: Option<usize>,
    context_key: Option<&str>)
                                -> Result<String, String> {
    let timeout = (timeout.unwrap_or(0) * 1000) as i32;
    let code_bytes = code.as_bytes().to_owned();
    let key_bytes = context_key.unwrap_or("").as_bytes().to_owned();
    t!(socket.write_i32::<NativeEndian>(timeout));
    t!(socket.write_i32::<NativeEndian>(key_bytes.len() as i32));
    t!(socket.write_i32::<NativeEndian>(code_bytes.len() as i32));
    t!(socket.write_all(&key_bytes));
    t!(socket.write_all(&code_bytes));
    t!(socket.flush());

    let result_len = t!(socket.read_i32::<NativeEndian>()) as usize;
    if result_len > 1024 * 1024 {
        Err("response from child too large".to_owned())
    } else {
        let mut result_bytes = Vec::with_capacity(result_len);
        unsafe {
            result_bytes.set_len(result_len);
        }
        t!(socket.read_exact(&mut result_bytes));
        Ok(String::from_utf8_lossy(&result_bytes).into_owned())
    }
}
