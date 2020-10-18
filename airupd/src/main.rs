#[macro_use]
extern crate ini;

use ansi_term::Color::*;
use libc::{sigfillset, sigprocmask, sigset_t, SIG_BLOCK};
use nng::*;
use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::env;
use std::fs::read_dir;
use std::mem;
use std::path::Path;
use std::process::exit;
use std::process::Command;
use std::sync::Mutex;
use std::thread;
use std::thread::JoinHandle;

static VERSION: &str = "0.1";
static MONITORS: Lazy<Mutex<HashMap<String, JoinHandle<()>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
#[cfg(not(feature = "airupdbg"))]
static AIRUP_CFG: &str = "/etc/airup.conf";
#[cfg(feature = "airupdbg")]
static AIRUP_CFG: &str = "./test/airup.conf";
fn get_target_dir(ad: &str, name: &str) -> String {
    let mut rslt = ad.to_string();
    if !ad.ends_with("/") {
        rslt.push('/');
    }
    rslt.push_str(name);
    rslt.push('/');
    rslt
}
fn open_airupctl_server() {
    println!(
        "{}Creating Airup Controlling Handling Server(NNG Rep)...",
        Green.paint(" * ")
    );
    let addr = "ipc://airupd".to_string();
    let server = Socket::new(Protocol::Rep0);
    server.listen(&addr[..]);
}
fn target_exec(td: &str) {
    let dir = td.to_string();
    println!(
        "{}Switching target to {}...",
        Blue.paint(" * "),
        Green.paint(td.clone())
    );
    if !Path::new(td.clone()).exists() {
        println!(
            "{}Failed to switch target to {}: file does not exist!",
            Red.paint(" * "),
            Green.paint(td.clone())
        );
        return;
    }
    let mut f = dir.clone();
    f.push_str("target.conf");
    let z = &f;
    let i = ini!(safe z);
    let i = match i {
        Ok(a) => a,
        Err(b) => {
            println!(
                "{}Target main configuration parse error(path: {})",
                Red.paint(" * "),
                f
            );
            eprintln!("{}{}", Red.paint(" * "), b);
            return;
        }
    };
    let name = tget(&i, "name");
    match name {
        Some(a) => println!(
            "{}Switched target to {}!",
            Green.paint(" * "),
            Green.paint(a)
        ),
        None => println!(
            "{}Switched target to {}!",
            Green.paint(" * "),
            Green.paint(&dir)
        ),
    };
    let dependencies = tget(&i, "deps");
    let dependencies = match dependencies {
        Some(a) => a,
        None => String::new(),
    };
    if dependencies != String::new() {
        let dependencies: Vec<&str> = dependencies.split(' ').collect();
        for i in dependencies.iter() {
            let mut t = dir.clone();
            t.push_str("../");
            target_exec(&get_target_dir(&t[..], i));
        }
    }
    let files = read_dir(&dir).unwrap();
    let mut fls: Vec<String> = Vec::new();
    for e in files {
        let d = e.unwrap();
        fls.push(d.file_name().to_str().unwrap().to_string());
    }
    for svc in fls.iter() {
        if !svc.ends_with(".svc") {
            continue;
        }
        let mut s = dir.clone();
        s.push_str(&svc);
        let s = &s[..];
        let _ini = ini!(s);
        let nn = svc.replace(".svc", &String::new()[..]);
        (*MONITORS.lock().unwrap()).insert(
            nn,
            thread::spawn(move || {
                sexec_monitor(&_ini);
            }),
        );
    }
}
fn resolve_args(argv: &Vec<String>) -> String {
    let mut c: String = String::from("default");
    if argv.len() == 1 {
        return "default".to_string();
    }
    for i in argv.iter() {
        if i == "single" {
            return "single".to_string();
        } else {
            let re = Regex::new(r"(?x)target=(?P<t>\w*)").unwrap();
            if i.to_string() == argv[0] {
                continue;
            } else {
                let caps = re.captures(i);
                c = match caps {
                    Some(b) => b["t"].to_string(),
                    None => "default".to_string(),
                };
            }
        }
    }
    c
}
fn print_svc_prompt(name: &str, desc: &str) {
    println!(
        "{}Starting {}({})...",
        Green.paint(" * "),
        Green.paint(name),
        Blue.paint(desc)
    );
}
fn sexec_monitor(s: &HashMap<String, HashMap<String, Option<String>>>) {
    match sget(s, "name") {
        Some(name) => {
            let desc = match sget(s, "desc") {
                Some(d) => d,
                None => "".to_string(),
            };
            print_svc_prompt(&name, &desc);
        }
        None => {
            print_svc_prompt("service", "");
        }
    };
}
fn tget(s: &HashMap<String, HashMap<String, Option<String>>>, c: &str) -> Option<String> {
    if s.get("target") == None {
        return None;
    }
    match &s["target"].get(c.clone()).clone() {
        Some(a) => Some(a.as_ref().unwrap().to_string()),
        None => None,
    }
}
fn sget(s: &HashMap<String, HashMap<String, Option<String>>>, c: &str) -> Option<String> {
    if s.get("svc") == None {
        return None;
    }
    match &s["svc"].get(c.clone()).clone() {
        Some(a) => Some(a.as_ref().unwrap().to_string()),
        None => None,
    }
}
fn cget(s: &HashMap<String, HashMap<String, Option<String>>>, c: &str) -> Option<String> {
    if s.get("rc") == None {
        return cget_default(c.clone());
    }
    match &s["rc"].get(c.clone()).clone() {
        Some(a) => Some(a.as_ref().unwrap().to_string()),
        None => cget_default(c.clone()),
    }
}
fn cget_default(c: &str) -> Option<String> {
    if c == "airup_dir" {
        Some(String::from("/etc/airup.d"))
    } else if c == "distro" {
        Some(String::from("Unknown UNIX"))
    } else {
        None
    }
}
fn main() {
    #[cfg(not(feature = "airupdbg"))]
    {
        if nix::unistd::getpid().as_raw() != 1 {
            println!("{}This program only runs as PID 1.", Red.paint(" * "));
            println!(
                "{}If you are trying to control Airup, use 'airupctl'.",
                Red.paint(" * ")
            );
            exit(-1);
        }
    }
    unsafe {
        let mut s1: sigset_t = mem::zeroed();
        let mut s2: sigset_t = mem::zeroed();
        sigfillset(&mut s1 as *mut sigset_t);
        sigprocmask(
            SIG_BLOCK,
            &mut s1 as *mut sigset_t,
            &mut s2 as *mut sigset_t,
        );
    }
    let cfgset = ini!(safe AIRUP_CFG);
    let cfgset = match cfgset {
        Ok(a) => a,
        Err(_b) => {
            println!(
                "{}: airup main config file not found!!!",
                Red.paint("error")
            );
            println!("Now airup will spawn /bin/sh. You can try to fix. Good luck!");
            let sh = Command::new("/bin/sh").spawn();
            match sh {
                Ok(mut a) => a.wait().unwrap(),
                Err(_b) => {
                    println!("\nFailed to spawn /bin/sh.");
                    loop {}
                }
            };
            main();
            exit(0);
        }
    };
    let airup_dir = cget(&cfgset, "airup_dir").unwrap();
    #[cfg(feature = "airupdbg")]
    {
        println!("Airup Directory: {}", &airup_dir);
    }
    println!(
        "{} {} is starting {}...",
        Blue.paint("Airup"),
        VERSION,
        Green.paint(cget(&cfgset, "distro").unwrap())
    );
    let argv: Vec<String> = env::args().collect();
    let target = resolve_args(&argv);
    target_exec(&get_target_dir(&airup_dir, &target));
}
