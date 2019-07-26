# Rust code evaluator

Like [rust-playpen](https://github.com/rust-lang/rust-playpen), but in Rust.

This repository contains:

* `evalbotlib/`: the evaluation backend
* `tgbot/`: the Telegram frontend
* `evaluators/`: some glue code for various REPLs
* `run/`: example configuration files and a script to set up a sandbox in Arch

## "Persistent" evaluator protocol

All integers are in little-endian byte order.

The daemon will be passed a Unix socket via FD 3. It should listen on the socket for connections.

Each request will come as a separate connection, and the bot will send the following:

| Field | Type | Description |
| ----- | ---- | ----------- |
| Timeout | Int32 | Timeout in milliseconds, or 0 for none |
| Context key length | Int32 | Context key in bytes |
| Code length | Int32 | Code length in bytes |
| Context key | UTF-8 string | Key of the context to use |
| Code | UTF-8 string | The code to evaluate |

The bot expects, for each request:

| Field | Type | Description |
| ----- | ---- | ----------- |
| Response length | Int32 | Response length in bytes |
| Response | UTF-8 string | The response |

Note that an evaluator will be killed by the bot if it doesn't respond within `timeout` seconds. (This means that you don't actually need to apply the timeout yourself.)
