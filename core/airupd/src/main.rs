#![allow(unused)]

use ansi_term::Color::*;
use libc::{pid_t, getpid, sigprocmask, sigset_t, sigfillset, SIG_BLOCK};
use std::process::exit;
use std::fs;
use std::process::Command;
use toml::Value;

static AIRUP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "airupdbg")]
static AIRUP_CFG: &str = "test/airup.conf";

#[cfg(not(feature = "airupdbg"))]
static AIRUP_CFG: &str = "/etc/airup.conf";

fn getpid_s() -> pid_t {
	unsafe {
		getpid()
	}
}
fn errormsg_not_pid1() {
	println!("{}This program must be run as PID 1.", Red.paint(" * "));
	println!("{}If you want to run this as other PIDs, build with 'airupdbg' feature.", Yellow.paint(" * "));
}
fn sigfillset_s(sigset: *mut sigset_t) {
	unsafe {
		sigfillset(sigset);
	}
}
fn sigprocmask_s(fc: i32, sig1: *mut sigset_t, sig2: *mut sigset_t) {
	unsafe {
		sigprocmask(fc, sig1, sig2);
	}
}
fn disable_signals() {
	let mut sig1: sigset_t = unsafe {std::mem::zeroed()};
	let mut sig2: sigset_t = unsafe {std::mem::zeroed()};
	sigfillset_s(&mut sig1 as *mut sigset_t);
	sigprocmask_s(SIG_BLOCK, &mut sig1 as *mut sigset_t, &mut sig2 as *mut sigset_t);
}
fn serious_err() -> ! {
    eprintln!("{}Airup Panicked and an emergency shell will be spawned.", Red.paint(" * "));
    let _airup001 = Command::new("/bin/sh")
        .spawn()
        .is_ok();
	loop {}
}
fn get_airup_configset() -> Value {
    let cfgstr = fs::read_to_string(AIRUP_CFG);
    match cfgstr {
    	Ok(a) => {
    		let x = a.parse::<Value>();
    		if x.is_err() {
    			serious_err();
    		}
    		return x.unwrap();
    	},
    	Err(b) => {
    		eprintln!("{}{}", Red.paint(" * "), b);
    		serious_err();
    	},
    };
}
#[tokio::main]
async fn main() {
    #[cfg(not(feature = "airupdbg"))]
    {
    	if getpid_s() != 1 {
    	    errormsg_not_pid1();
    		exit(-1);
    	}
    	disable_signals();
    }
    let airup_cfgset = get_airup_configset();
}
