use ansi_term::Color::*;
use libc::{pid_t, getpid, sigprocmask, sigset_t, sigfillset, SIG_BLOCK};
use std::process::exit;
use tokio::fs;
use tokio::process::Command;
use toml::Value;
use std::collections::HashMap;
use nng::*;
use std::path::PathBuf;
use tokio::sync::Mutex;
use once_cell::sync::Lazy;
use std::result::Result;
use std::thread;

enum SvcOperate {
	Start(String),
	Restart(String),
	Stop(String),
	Status(String),
}
enum AirupMsg {
	Svc(SvcOperate),
	TargetExec(String),
	Shutdown(String),
}
static RUNNING_SERVICES: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
static AIRUP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(feature = "airupdbg")]
static AIRUP_CFG: &str = "test/airup.conf";

#[cfg(not(feature = "airupdbg"))]
static AIRUP_CFG: &str = "/etc/airup.conf";

async fn getmsg(msg: &str) -> Option<AirupMsg> {
	let mut msg = msg.to_string();
	let returning: AirupMsg;
	if msg.starts_with("svc ") {
	    let opr: SvcOperate;
		msg.remove(0);
		msg.remove(1);
		msg.remove(2);
		msg.remove(3);
		if msg.starts_with("start ") {
			msg.remove(0);
			msg.remove(1);
			msg.remove(2);
			msg.remove(3);
			msg.remove(4);
			msg.remove(5);
			opr = SvcOperate::Start(msg);
		} else if msg.starts_with("stop ") {
			msg.remove(0);
			msg.remove(1);
			msg.remove(2);
			msg.remove(3);
			msg.remove(4);
			opr = SvcOperate::Stop(msg);
		} else {
			return None;
		}
		returning = AirupMsg::Svc(opr);
	} else {
		return None;
	}
	Some(returning)
}
fn ipc_loop() -> ! {
    let address = "tcp://localhost:61257";
    let server = Socket::new(Protocol::Rep0);
    if server.is_err() {
    	ipc_loop();
    }
    let server = server.unwrap();
    let listen_stat = server.listen(address);
    if listen_stat.is_err() {
        drop(server);
    	ipc_loop();
    }
    println!("{}Opening Airup's IPC Socket 61257...", Green.paint(" * "));
	loop {
		let msg = server.recv();
	}
}
fn ipc_par() {
	thread::spawn( || {
		ipc_loop();
	} );
}
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
fn apanic() {
	eprintln!("failed");
	loop {}
}
async fn serious_err() {
    eprintln!("{}Airup Panicked and an emergency shell will be spawned.", Red.paint(" * "));
    let _t = Command::new("/bin/sh")
        .spawn()
        .is_err();
    if _t {
    	apanic();
    }
}
async fn get_airup_configset() -> Value {
    let cfgstr = fs::read_to_string(AIRUP_CFG).await;
    match cfgstr {
    	Ok(a) => {
    		let x = a.parse::<Value>();
    		if x.is_err() {
    			serious_err().await;
    		}
    		return x.unwrap();
    	},
    	Err(b) => {
    		eprintln!("{}{}", Red.paint(" * "), b);
    		serious_err().await;
    		loop {}
    	},
    };
}
fn get_airupcfg_default(tname: &str) -> Value {
	let mut matches: HashMap<String, Value> = HashMap::new();
	matches.insert("airup_dir".to_string(), Value::String("/etc/airup.d".to_string()));
	matches.insert("distro_name".to_string(), Value::String("Unknown OS".to_string()));
	matches.insert("enable_logging".to_string(), Value::Boolean(true));
	matches[tname].clone()
}
async fn sgac(cs: &Value, tname: &str) -> Value {
	let cs = cs.clone();
	let atable = cs.get("airup");
	let atable = match atable {
		Some(a) => a,
		None => {
			eprintln!("{}Failed to get Airup segment in config file.", Red.paint(" * "));
			serious_err().await;
			loop {}
		},
	};
	atable.get(tname.clone()).unwrap_or(&get_airupcfg_default(tname)).clone()
}
async fn supervisor_loop(dir: &str) -> Result<(), ()> {
	let mut svctoml = PathBuf::from(dir);
	svctoml.push("svc.toml");
	if !svctoml.as_path().exists() {
		return Err(());
	}
	let svctoml = fs::read_to_string(svctoml.to_str().unwrap()).await;
	let svctoml = match svctoml {
		Ok(a) => {
			let x = a.parse::<Value>();
			if x.is_err() {
				return Err(());
			} else {
				x
			}
		},
		Err(b) => {
		    eprintln!("{}{}", Red.paint(" * "), b);
			return Err(());
		}
	};
	Ok(())
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
    let airup_cfgset = get_airup_configset().await;
    println!("Airup {} is launching {}...", Blue.paint(AIRUP_VERSION), Blue.paint(sgac(&airup_cfgset, "distro_name").await.as_str().unwrap()));
    ipc_par();
	let airup_dir = sgac(&airup_cfgset, "airup_dir").await.as_str().unwrap().to_string();
	let mut etdir = PathBuf::from(&airup_dir);
}
