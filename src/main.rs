use std::process;

fn main() {
    if let Err(e) = lumiere::run() {
        eprintln!("\x1b[91m\rerror:\x1b[0m {e}");
        process::exit(1);
    }
}
