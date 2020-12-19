use ansi_term::Color::*;
use nng::{Protocol, Socket};
use std::{env, fs};

fn main() {
    fs::create_dir("/tmp/airupoe").unwrap();
    env::set_current_dir("/tmp/airupoe").unwrap();
    let server = Socket::new(Protocol::Pull0).unwrap();
    server.listen("ipc://airupoe").unwrap();
    println!("{}Starting Airup-of-Events...", Green.paint(" * "));
}
