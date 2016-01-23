# Rust IRC evaluator

Like [rust-playpen](https://github.com/rust-lang/rust-playpen), but in Rust.

This repository contains:

* `evalbot/`: the bot itself
* `evaluators/`: some glue code for C# (Mono) and Python REPLs
* `run/`: example configuration files and a script to set up a sandbox in Arch

## Simple usage

1. Compile the bot.

   ````
   $ pushd evalbot; cargo build --release; popd
   ````

2. Set up the sandbox.

   ````
   $ pushd run; sudo ./init.sh; popd
   ````

   N.B. `init.sh` is written for a system with `pacman`.

3. Edit the configuration files

   ````
   $ pushd run
   $ cp evalbot.toml.in evalbot.toml; cp evalbot.irc.toml.in evalbot.irc.toml
   $ $EDITOR evalbot.toml
   $ $EDITOR evalbot.irc.toml
   $ popd
   ````

4. Run the bot

   ````
   $ pushd run
   $ cp ../evalbot/target/release/evalbot .
   $ ./evalbot
   ````

5. Try it out!

   ````
   <$you> rs>println!("Hello!");
   <$bot> >Hello!
   <$bot> >()
   ````

## Advanced usage

This bot depends on [playpen](https://github.com/thestinger/playpen).

If you wish to use some other means of sandboxing evaluated code (or not sandbox it at all), you can edit the code in `evalbot/src/playpen.rs`. However, note that the bot depends on playpen to enforce the timeout for non-daemon evaluators (i.e. `persistent = false`). Check the code if you're still unsure.

The chroot sandbox just needs to be able to run whatever you configure it to run (in `evalbot.toml`).

## "Persistent" evaluator protocol

The bot will send, for each request, via standard input:

| Field | Type | Description |
| ----- | ---- | ----------- |
| Timeout | Int32 | Timeout in milliseconds, or 0 for none |
| Context key length | Int32 | Context key in bytes |
| Code length | Int32 | Code length in bytes |
| Context key | UTF-8 string | Key of the context to use |
| Code | UTF-8 string | The code to evaluate |

The bot expects, for each request, from standard output:

| Field | Type | Description |
| ----- | ---- | ----------- |
| Response length | Int32 | Response length in bytes |
| Response | UTF-8 string | The response |

Note that an evaluator will be killed by the bot if it doesn't respond within `1.5 * timeout` seconds.
