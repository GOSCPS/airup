use ansi_term::Color::*;
use ini::ini;
use libc::{kill, pid_t, sigfillset, sigprocmask, sigset_t, SIGTERM, SIG_BLOCK};
use nng::*;
use once_cell::sync::Lazy;
use regex::Regex;
use std::cmp::PartialEq;
use std::collections::HashMap;
use std::env;
use std::fs::read_dir;
use std::mem;
use std::path::Path;
use std::process::exit;
use std::process::Child;
use std::process::Command;
use std::result::Result;
use std::sync::Mutex;
use std::thread;

static VERSION: &str = "0.3";
static SVCMSGS: Lazy<Mutex<HashMap<String, SvcMsg>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static AIRUP_DIR: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

#[cfg(not(feature = "airupdbg"))]
static AIRUP_CFG: &str = "/etc/airup.conf";

#[cfg(feature = "airupdbg")]
static AIRUP_CFG: &str = "./test/airup.conf";

#[derive(PartialEq)]
enum SvcMsg {
    Restart,
    Stop,
    Running,
    Readying,
    MonitorExit,
}
fn svc_running(n: &str) -> bool {
    let lk = &mut *SVCMSGS.lock().unwrap();
    let t = match lk.get(n) {
        Some(_a) => true,
        None => false,
    };
    if t == true {
        match lk.get(n).unwrap() {
            SvcMsg::Stop => lk.insert(n.to_string(), SvcMsg::Running),
            SvcMsg::Readying => loop {
                if *(*SVCMSGS.lock().unwrap()).get(n).unwrap() == SvcMsg::Running {
                    break None;
                }
            },
            _ => None,
        };
    }
    t
}
fn get_target_dir(ad: &str, name: &str) -> String {
    let mut rslt = ad.to_string();
    if !ad.ends_with("/") {
        rslt.push('/');
    }
    rslt.push_str(name);
    rslt.push('/');
    rslt
}
fn msghandler(msg: &str) -> Result<String, ()> {
    if msg.starts_with("service") {
        let msg = msg;
        let msg: Vec<&str> = msg.split(" ").collect();
        let action = *msg.get(1).unwrap_or(&"");
        let svc = *msg.get(2).unwrap_or(&"");
        if action == "" || svc == "" {
            Err(())
        } else {
            let a: SvcMsg;
            if action == "start" {
                a = SvcMsg::Running;
            } else if action == "stop" {
                a = SvcMsg::Stop;
            } else if action == "restart" {
                a = SvcMsg::Restart;
            } else if action == "status" {
                let lk = &*SVCMSGS.lock().unwrap();
                if lk.get(svc.clone()).is_none() {
                    return Ok("SvcNotExist".to_string());
                } else {
                    let st = lk.get(svc).unwrap();
                    let st = match st {
                        SvcMsg::Running => "Running",
                        SvcMsg::Stop => "Stop",
                        _ => "Working",
                    };
                    return Ok(st.to_string());
                }
            } else {
                return Err(());
            }
            let lk = &mut *SVCMSGS.lock().unwrap();
            if lk.get(svc.clone()).is_none() && a != SvcMsg::Running {
                return Ok(String::from("SvcNotExist"));
            }
            if a == SvcMsg::Running {
                let mut tmp = String::from(svc);
                tmp.push_str(".svc");
                sexec(
                    &get_target_dir(&*AIRUP_DIR.lock().unwrap(), "svc"),
                    &tmp[..],
                );
                return Ok("Ok".to_string());
            }
            lk.insert(svc.to_string(), a);
            return Ok(String::from("Ok"));
        }
    } else if msg.starts_with("target") {
        Err(())
    } else {
        Err(())
    }
}
fn open_airupctl_server() {
    println!(
        "{}Creating Airup Controlling Handling Server(NNG Rep)...",
        Green.paint(" * ")
    );
    let addr = "tcp://localhost:11257".to_string();
    let server = Socket::new(Protocol::Rep0);
    let server = match server {
        Ok(a) => a,
        Err(b) => {
            eprintln!("{}{}", Red.paint(" * "), b);
            println!(
                "{}Airup Controlling Handling Server creating failed: recreating...",
                Red.paint(" * ")
            );
            open_airupctl_server();
            return;
        }
    };
    let t = server.listen(&addr[..]);
    match t {
        Ok(_a) => (),
        Err(b) => {
            eprintln!("{}{}", Red.paint(" * "), b);
            println!(
                "{}Airup Controlling Handling Server creating failed: recreating...",
                Red.paint(" * ")
            );
            open_airupctl_server();
            return;
        }
    };
    loop {
        let msg = server.recv();
        if msg.is_ok() {
            let msg = msg.unwrap();
            let msg_str = msg.as_slice();
            let msg_str = String::from_utf8_lossy(msg_str);
            let msg_str = msghandler(&msg_str[..]);
            let msg = match msg_str {
                Ok(a) => a,
                Err(()) => "Failed".to_string(),
            };
            let _rslt = server.send(msg.as_bytes()).is_ok();
        }
    }
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
        sexec(&dir, &svc);
    }
}
fn sexec(dir: &str, svc: &str) {
    let mut s = dir.to_string().clone();
    s.push_str(&svc);
    let s = &s[..];
    let _ini = ini!(s);
    let nn = svc.replace(".svc", &String::new()[..]);
    if svc_running(&nn) {
        return;
    }
    (*SVCMSGS.lock().unwrap()).insert(nn.clone(), SvcMsg::Readying);
    thread::spawn(move || {
        sexec_monitor(&_ini, &nn);
    });
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
fn rcmdln(s: &str) -> (String, Vec<String>) {
    if s.contains(" ") {
        let t: Vec<&str> = s.split(" ").collect();
        let head = t[0].clone().to_string();
        let mut n: usize = 0;
        let mut r: Vec<String> = Vec::new();
        for i in t.iter() {
            if n == 0 {
                n = 1;
                continue;
            }
            n += 1;
            r.push(i.to_string());
        }
        return (head, r);
    } else {
        return (s.to_string(), Vec::new());
    }
}
fn rsystem(s: &str) -> Option<Child> {
    let (k, b) = rcmdln(s);
    let handler = Command::new(k).args(&b).spawn();
    match handler {
        Ok(a) => Some(a),
        Err(_b) => None,
    }
}
fn stopchild(c: &mut Child) {
    unsafe {
        kill(c.id() as pid_t, SIGTERM);
    }
    std::thread::sleep(std::time::Duration::from_millis(5000));
    if c.try_wait().is_err() {
        let z = c.kill();
        if z.is_err() {
            eprintln!("{}Failed to send SIGKILL to process!", Red.paint(" * "));
        }
    }
}
fn child_running(c: &mut Child, n: &str) -> bool {
    if c.try_wait().is_err() {
        return true;
    } else {
        if c.try_wait().unwrap().is_none() {
            return false;
        } else {
            if !c.try_wait().unwrap().unwrap().success() {
                println!(
                    "{}Service {} executing failed!!!",
                    Red.paint(" * "),
                    Red.paint(n.clone())
                );
            } else {
                println!(
                    "{}Service {} didn't report an error, but exited.",
                    Yellow.paint(" * "),
                    Yellow.paint(n.clone())
                );
            }
            return false;
        }
    }
}
fn sexec_monitor(s: &HashMap<String, HashMap<String, Option<String>>>, id: &str) {
    let deps = match sget(s, "deps") {
        Some(a) => a,
        None => String::new(),
    };
    if deps != String::new() {
        let a_d = &*AIRUP_DIR.lock().unwrap().clone();
        let pop: Vec<&str> = deps[..].split(" ").collect();
        for i in pop.iter() {
            if !svc_running(&i) {
                let airup_dir = a_d.clone();
                let mut path = i.to_string();
                path.push_str(".svc");
                sexec(&get_target_dir(&airup_dir, "svc"), &path);
            }
        }
    }
    let cmd = match sget(s, "exec") {
        Some(a) => a,
        None => {
            println!("{}Service description unavailable!", Red.paint(" * "));
            return;
        }
    };
    let child = rsystem(&cmd[..]);
    let mut child = child.unwrap();
    match sget(s, "capture") {
        Some(ra) => {
            if ra != "false" {
                child.stdout.take();
            }
        }
        None => {
            child.stdout.take();
        }
    }
    match sget(s, "name") {
        Some(name) => {
            let desc = match sget(s, "desc") {
                Some(d) => d,
                None => "".to_string(),
            };
            print_svc_prompt(&name, &desc);
        }
        None => {
            print_svc_prompt(
                id.clone(),
                &sget(s, "desc").unwrap_or("comes without description".to_string()),
            );
        }
    };
    (*SVCMSGS.lock().unwrap()).insert(id.clone().to_string(), SvcMsg::Running);
    let mut rt = 0;
    let rtmax = sget(s, "retry")
        .unwrap_or("3".to_string())
        .parse::<isize>()
        .unwrap_or(3);
    let mut optiu = false;
    let howtostop = sget(s, "stop_handler").unwrap_or("default".to_string());
    let howtorestart = sget(s, "restart_handler").unwrap_or("default".to_string());
    let mut stopped = false;
    loop {
        let lk = &*SVCMSGS.lock().unwrap();
        let lk = lk.get(&id.clone().to_string()).unwrap();
        match lk {
            SvcMsg::Stop => {
                if stopped == true {
                    continue;
                }
                if howtostop != "default" {
                    rsystem(&howtostop[..]);
                } else {
                    stopchild(&mut child);
                }
                stopped = true;
            }
            SvcMsg::Restart => {
                stopped = false;
                if howtostop != "default" {
                    rsystem(&howtorestart[..]);
                    rsystem(&howtostop[..]);
                } else {
                    stopchild(&mut child);
                }
                *&mut optiu = false;
                *&mut rt = 0;
                (*SVCMSGS.lock().unwrap()).insert(id.clone().to_string(), SvcMsg::Running);
            }
            SvcMsg::MonitorExit => {
                return;
            }
            SvcMsg::Running => {
                stopped = false;
                if rt == rtmax {
                    if !*&optiu {
                        println!(
                            "{}Service {} restarted too many times!",
                            Red.paint(" * "),
                            Red.paint(id.clone())
                        );
                        *&mut optiu = true;
                    }
                } else {
                    let cld = &mut child;
                    if !child_running(cld, id.clone()) {
                        let tt = rsystem(&cmd[..]);
                        *cld = tt.unwrap();
                        *&mut rt += 1;
                    }
                }
            }
            _ => (),
        };
    }
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
    (*AIRUP_DIR.lock().unwrap()) = airup_dir.clone();
    println!(
        "{} {} is starting {}...",
        Blue.paint("Airup"),
        VERSION,
        Green.paint(cget(&cfgset, "distro").unwrap())
    );
    let argv: Vec<String> = env::args().collect();
    let target = resolve_args(&argv);
    target_exec(&get_target_dir(&airup_dir, &target));
    open_airupctl_server();
}
