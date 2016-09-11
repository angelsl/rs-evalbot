extern crate gcc;

fn main() {
    println!("cargo:rustc-link-lib=static=kcgi");
    gcc::compile_library("libtgbotc.a", &["src/main.c"]);
}
