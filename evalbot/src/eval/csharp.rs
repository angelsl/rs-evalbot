use playpen;
use std::process::Child;
use eval::persistent;

fn spawn_child(sandbox: &str) -> Child {
    playpen::spawn(sandbox, "/usr/bin/mono", "mono_syscalls",
                   &["/usr/local/bin/cseval.exe"],
                   None,
                   false).unwrap()
}

pub fn evaluator(sandbox: &str) -> persistent::PersistentEvaluator {
    let sandbox = sandbox.to_owned();
    persistent::new(move || { spawn_child(&sandbox) })
}

/*         if code.contains("OpenStandardOutput")
            || code.contains("OpenStandardInput")
                || code.contains("OpenStandardError") {
                    // prevent evaluated code from hijacking the stream :/
                    return Err("don't use Console.OpenStandard*".to_owned());
                } */
