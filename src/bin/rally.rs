#[path = "../client/mod.rs"]
mod client;

fn main() {
    if let Err(error) = client::main() {
        eprintln!("rally: {error:#}");
        std::process::exit(1);
    }
}
