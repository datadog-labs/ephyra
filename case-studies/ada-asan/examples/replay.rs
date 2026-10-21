//! Replays target/crash.bin (or the file given) under the sanitizers.
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "target/crash.bin".to_owned());
    ada_asan::replay(&std::fs::read(path).unwrap());
    eprintln!("no failure");
}
