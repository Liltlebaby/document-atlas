#![allow(unexpected_cfgs, dead_code, unused_imports)]
include!("lib.rs");

#[tokio::main]
async fn main() {
    if let Err(error) = web::run().await {
        eprintln!("Document Atlas Web: {error}");
        std::process::exit(1);
    }
}
