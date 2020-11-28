use ansi_term::Color::*;
use libc::{getpid, kill, pid_t, sigfillset, sigprocmask, sigset_t, uid_t, SIG_BLOCK, waitpid, WNOHANG, c_int, SIGKILL, SIGTERM};
use nng::{Protocol, Socket};
use once_cell::sync::Lazy;
use std::{
    cmp::PartialEq,
    collections::HashMap,
    convert::TryInto,
    env,
    fmt::{Display, Formatter},
    fs, io, mem, panic, time,
    path::{Path, PathBuf},
    process::{exit, Command},
    sync::{RwLock, Arc},
    thread::{Builder, sleep},
};
use toml::{map::Map, Value};

enum User {
    Id(uid_t),
    Name(String),
}
impl Display for User {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            User::Id(id) => write!(f, "{}", id.to_string()),
            User::Name(name) => write!(f, "{}", name),
        }
    }
}
#[derive(PartialEq, Copy, Clone)]
enum SvcStatus {
    Readying,
    Running,
    Working,
    Stopped,
    Unmentioned,
}
enum Stage {
    PreStart,
    Milestones(String),
    Shutdown,
    CtrlAltDel,
}

static COMM_INIT: Lazy<RwLock<bool>> = Lazy::new(|| RwLock::new(false));
static CURRENT_STAGE: Lazy<RwLock<Stage>> = Lazy::new(|| RwLock::new(Stage::PreStart));
static AIRUP_VERSION: &str = env!("CARGO_PKG_VERSION");
static SVC_STATUS: Lazy<RwLock<HashMap<String, SvcStatus>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));
static DEFAULT_CFG: Lazy<HashMap<String, Value>> = Lazy::new(|| {
    let mut a: HashMap<String, Value> = HashMap::new();
    a.insert("airup/prestart_paral".to_string(), Value::Boolean(false));
    a.insert(
        "airup/osname".to_string(),
        Value::String("Unknown OS".to_string()),
    );
    a.insert(
        "airup/airup_home".to_string(),
        Value::String("/etc/airup.d".to_string()),
    );
    let mut default_prestart_dir = a["airup/airup_home"].clone().as_str().unwrap().to_string();
    default_prestart_dir.push_str("/prestart");
    a.insert(
        "airup/env_path".to_string(),
        Value::String("DONT_SETUP".to_string()),
    );
    a.insert(
        "internal/prestart_dir".to_string(),
        Value::String(default_prestart_dir),
    );
    a.insert(
        "milestone/description".to_string(),
        Value::String("An airup milestone".to_string()),
    );
    a.insert("milestone/paral".to_string(), Value::Boolean(true));
    a.insert("milestone/env_list".to_string(), Value::Table(Map::new()));
    a.insert("milestone/dependencies".to_string(), Value::Array(vec![]));
    a.insert(
        "svc/description".to_string(),
        Value::String("An airup service".to_string()),
    );
    a.insert("svc/env_list".to_string(), Value::Table(Map::new()));
    a.insert("svc/take_io".to_string(), Value::Boolean(true));
    a.insert("svc/dependencies".to_string(), Value::Array(Vec::new()));
    a.insert("svc/stop_way".to_string(), Value::Integer(15));
    a.insert("svc/cleanup_on_restart".to_string(), Value::Boolean(true));
    a.insert("svc/retry_time".to_string(), Value::Integer(3));
    a.insert("svc/kill_timeout".to_string(), Value::Integer(5000));
    a
});

#[cfg(not(feature = "quickdbg"))]
static AIRUP_CONF: &str = "/etc/airup.conf";

#[cfg(feature = "quickdbg")]
static AIRUP_CONF: &str = "debug/airup.conf";

// Start Service Suoervisor
fn svc_running_core(id: &str) -> SvcStatus {
    *(*SVC_STATUS.read().unwrap())
        .get(id)
        .unwrap_or(&SvcStatus::Unmentioned)
}
fn svc_running_block(id: &str) -> bool {
    if svc_running_core(id.clone()) == SvcStatus::Readying || svc_running_core(id.clone()) == SvcStatus::Working {
        loop {
            if svc_running_core(id) == SvcStatus::Running {
                return true;
            }
        }
    } else if svc_running_core(id.clone()) == SvcStatus::Running {
        return true;
    } else if svc_running_core(id.clone()) == SvcStatus::Stopped {
        return true;
    }
    false
}
fn regsvc(id: &str, stat: SvcStatus) {
    (*SVC_STATUS.write().unwrap()).insert(id.to_string(), stat);
}
fn regmsg(id: &str) -> Socket {
    let base_dir = "inproc://airup/supervisors/";
    let mut dir = String::from(base_dir);
    dir.push_str(id.clone());
    let skt = Socket::new(Protocol::Pair1).unwrap();
    skt.listen(&dir).unwrap();
    let telr = Socket::new(Protocol::Push0).unwrap();
    telr.dial("inproc://airup/regsvc").unwrap();
    let mut msg = String::from("up ");
    msg.push_str(id);
    telr.send(msg.as_bytes()).unwrap();
    skt
}
fn delmsg(id: &str) {
    let telr = Socket::new(Protocol::Push0).unwrap();
    telr.dial("inproc://airup/regsvc").unwrap();
    let mut msg = String::from("down ");
    msg.push_str(id);
    telr.send(msg.as_bytes()).unwrap();
}
fn delsvc(id: &str) {
    (*SVC_STATUS.write().unwrap()).remove(id);
}
fn svcrun(airup_dir: &'static str, svctomlpath: &'static str) -> bool {
    let svctoml = get_toml_of(svctomlpath);
    if svctoml.is_none() {
        eprintln!(
            "{}Some problems happened when launching service {}.",
            Red.paint(" * "),
            svctomlpath
        );
        return false;
    }
    let svctoml = svctoml.unwrap();
    let id = svcid_detect(svctomlpath.clone());
    if svc_running_block(&id) {
        return true;
    }
    let thrd = Builder::new().name(id.to_string());
    let thrd = thrd.spawn(move || svc_supervisor_main(&id, airup_dir, svctoml));
    if thrd.is_err() {
        eprintln!("{}OS Error: Failed to create thread!", Red.paint(" * "));
        return false;
    }
    true
}
fn svc_dep(airup_dir: &'static str, deps: Vec<String>) {
    for dep in deps {
        let mut i = String::from(&dep);
        if i.starts_with("alias::") {
            unimplemented!();
        } else {
            let mut e = PathBuf::from(airup_dir.clone());
            e.push("svc");
            i.push_str(".svc");
            e.push(&i);
            svcrun(
                airup_dir.clone(),
                Box::leak(e.to_string_lossy().to_string().into_boxed_str()),
            );
        }
        loop {
        	if svc_running_block(&dep) {
        		break;
        	}
        }
    }
}
fn g_svc(set: &Value, vid: &str) -> Option<Value> {
    let temp = tomlget(Some(set), "svc", vid.clone());
    let default = Lazy::new(|| get_default_value("svc", vid));
    if temp.is_none()
        && (vid != "prompt"
            && vid != "user"
            && vid != "action_user"
            && vid != "exec"
            && vid != "pid_file"
            && vid != "pre_exec"
            && vid != "pre_stop"
            && vid != "cleanup"
            && vid != "pre_restart"
            && vid != "ready_timeout"
            && vid != "restart_way")
    {
        return Some(default.as_ref().unwrap().clone());
    }
    if temp.is_none()
        && (vid == "prompt"
            || vid == "user"
            || vid == "action_user"
            || vid == "exec"
            || vid == "pid_file"
            || vid == "pre_exec"
            || vid == "pre_stop"
            || vid == "cleanup"
            || vid == "pre_restart"
            || vid == "ready_timeout"
            || vid == "restart_way")
    {
        return None;
    }
    let temp = temp.unwrap();
    if (!temp.is_bool()) && (vid == "take_io") {
        return Some(default.as_ref().unwrap().clone());
    }
    if (!temp.is_str()) && (vid == "description" || vid == "pre_exec" || vid == "exec") {
        return Some(default.as_ref().unwrap().clone());
    }
    if (!temp.is_array()) && (vid == "dependencies") {
        return Some(default.as_ref().unwrap().clone());
    }
    Some(temp)
}
fn envmaptostr(mp: &Map<String, Value>) -> String {
    let keys: Vec<&String> = mp.keys().collect();
    let mut r = String::new();
    for key in keys.iter() {
        let val = mp.get(key.clone()).unwrap();
        if !val.is_str() {
            continue;
        }
        let val = val.as_str().unwrap();
        r.push_str(key);
        r.push('=');
        r.push('"');
        r.push_str(val);
        r.push('"');
    }
    r
}
fn get_user_by_value(val: Option<Value>) -> User {
    if val.is_none() {
        return User::Id(0);
    }
    let val = val.unwrap();
    match val {
        Value::Integer(a) => User::Id(a.try_into().unwrap()),
        Value::String(b) => User::Name(b),
        _ => User::Id(0),
    }
}
fn svc_supervisor_main(id: &str, airup_dir: &'static str, svctoml: Value) {
    regsvc(id.clone(), SvcStatus::Readying);
    let channel = regmsg(id.clone());
    // Ready some basic values.
    let prompt = g_svc(&svctoml, "prompt").unwrap_or(Value::String(id.to_string()));
    let prompt = prompt.as_str().unwrap_or(id.clone()).to_string();
    let desc = g_svc(&svctoml, "description")
        .unwrap();
    let desc = desc .as_str()
        .unwrap()
        .to_string();
    let env_map = g_svc(&svctoml, "env_list").unwrap();
    let env_map = env_map.as_table().unwrap();
    let sh_env = envmaptostr(env_map);
    let user = g_svc(&svctoml, "user");
    let action_user = g_svc(&svctoml, "action_user");
    let user = get_user_by_value(user);
    let action_user = get_user_by_value(action_user);
    let take_io = g_svc(&svctoml, "take_io")
        .unwrap()
        .as_bool()
        .unwrap();
    let deps = g_svc(&svctoml, "dependencies").unwrap();
    let deps = deps.as_array().unwrap();
    let deps = vv_to_vs(deps.to_vec());
    svc_dep(airup_dir, deps);
    // Ready for exec
    let pre_exec = g_svc(&svctoml, "pre_exec");
    let exec = g_svc(&svctoml, "exec");
    let pid_file = g_svc(&svctoml, "pid_file");
    let exec = match exec {
        Some(a) => Box::leak(a.as_str().unwrap().to_string().into_boxed_str()),
        None => {
            eprintln!(
                "{}Failed to execute service {}: no 'exec' specified!",
                Red.paint(" * "),
                prompt
            );
            return;
        }
    };
    // Ready for stop
    let pre_stop = g_svc(&svctoml, "pre_stop");
    // stop_way: string to exec a command, number to send signal
    let stop_way = g_svc(&svctoml, "stop_way").unwrap();
    let cleanup = g_svc(&svctoml, "cleanup");
    // Ready for restart
    let pre_restart = g_svc(&svctoml, "pre_restart");
    let restart_way = g_svc(&svctoml, "restart_way").unwrap_or(stop_way.clone());
    let cleanup_on_restart = g_svc(&svctoml, "cleanup_on_restart").unwrap();
    let cleanup_on_restart = cleanup_on_restart.as_bool().unwrap();
    // exception handling data.
    let retry_time = g_svc(&svctoml, "retry_time").unwrap();
    let retry_time = retry_time.as_integer().unwrap();
    let ready_timeout = g_svc(&svctoml, "ready_timeout");
    let kill_timeout = g_svc(&svctoml, "kill_timeout").unwrap();
    let kill_timeout = kill_timeout.as_integer().unwrap();
    // ready functions
    let mut pid = 0;
    let id = String::from(id);
    let mut kill_timer: Option<Arc<RwLock<bool>>> = None;
    let mut die_timer: Option<Arc<RwLock<bool>>> = None;
    let mut full_stop = || {
    	let pre_stop = &pre_stop;
    	let stop_way = &stop_way;
        if pre_stop.is_some() {
            let pre_stop = pre_stop.as_ref().unwrap();
            let pre_pid = asystem(&action_user, pre_stop.as_str().unwrap(), &sh_env);
            if pre_pid.is_some() {
                wait(pre_pid.unwrap());
            }
        }
        let action_user = &action_user;
        let env = &sh_env;
        let pid = &pid;
        svc_stop(action_user, env, stop_way, pid.clone());
        let kill_timeout = &kill_timeout;
        *&mut kill_timer = Some(timer(time::Duration::from_millis(kill_timeout.clone().try_into().unwrap())).unwrap());
        regsvc(&id, SvcStatus::Stopped);
    };
    let full_exec = || {
        let pre_exec = &pre_exec;
        if pre_exec.is_some() {
            let pre_exec = pre_exec.as_ref().unwrap();
    	    let pre_pid = asystem(&action_user, pre_exec.as_str().unwrap(), &sh_env);
    	    if pre_pid.is_some() {
    		    wait(pre_pid.unwrap());
    	    }
        }
    	let pid = svc_exec(&user, &sh_env, &exec, take_io, &pid_file);
    	if pid.is_none() {
    		eprintln!("{}Failed to execute service {}!", Red.paint(" * "), Red.paint(prompt.clone()));
    		return 0;
    	} else {
    	    let ready_timeout = &ready_timeout;
    	    if ready_timeout.is_some() {
                let ready_timeout = ready_timeout.as_ref().unwrap();
                let ready_timeout = ready_timeout.as_integer().unwrap();
                let ready_timeout = match ready_timeout.try_into() {
                	Ok(a) => a,
                	Err(_) => {
                		eprintln!("{}{}: Timeout must be more than zero!", Red.paint(" * "), prompt.clone());
                        return 0;
                	},
                };
    		    sleep(time::Duration::from_millis(ready_timeout));
    	    }	
    	}
    	let pid = pid.unwrap();
    	pid
    };
    // startup first
    pid = full_exec();
    if pid == 0 {
        eprintln!("{}Failed to start service {}!", Red.paint(" * "), Red.paint(prompt.clone()));
    	return;
    }
    regsvc(&id, SvcStatus::Running);
    println!("{}Starting service {}({})...", Green.paint(" * "), Green.paint(prompt.clone()), Blue.paint(desc.clone()));
    // observe
    let mut retry_count = 0;
    let mut retry = true;
    loop {
        if svc_running_core(&id) == SvcStatus::Stopped {
            let kill_timer = &mut kill_timer;
            if kill_timer.is_some() {
                let kill_timer_unwrap = kill_timer.as_ref().unwrap();
            	if *kill_timer_unwrap.read().unwrap() {
            		*kill_timer = None;
            		if try_wait(pid).is_none() {
            			send_signal(pid, SIGKILL);
            		}
            	}
            	die_timer = Some(timer(time::Duration::from_secs(300)).unwrap());
            } else if die_timer.is_some() {
                let die_timer_unwrap = die_timer.as_ref().unwrap();
            	if *die_timer_unwrap.read().unwrap() {
            		delsvc(&id);
            		delmsg(&id);
            		return;
            	}
            }
        }
    	let msg = channel.try_recv();
    	if msg.is_ok() {
    		let msg = msg.unwrap();
    		let msg = String::from_utf8_lossy(&msg);
    		if msg == "pid" {
    			match channel.send(pid.to_string().as_bytes()) {
    				Ok(_) => (),
    				Err(_) => {
    					continue;
    				},
    			};
    		} else if msg == "down" {
    			full_stop();
    		} else if msg == "up" {
    			pid = full_exec();
    		}
    	}
    	let t = try_wait(pid);
    	if t.is_some() {
    	    let t = t.unwrap();
    	    if t == 0 && retry && !(retry_count == retry_time) {
    	    	eprintln!("{}Service {} stopped, but not returning an error. restarting...", Yellow.paint(" * "), Yellow.paint(prompt.clone()));
    	    } else if t !=0 && retry && !(retry_count == retry_time) {
    	    	eprintln!("{}Service {} stopped unexpectedly! restarting...", Red.paint(" * "), Red.paint(prompt.clone()));
    	    }
    	    if retry_count == retry_time && retry {
    	    	eprintln!("{}Service {} restarted too many times!", Red.paint(" * "), Red.paint(prompt.clone()));
                retry = false;
    	    } else if retry_count != retry_time && retry {
    	    	retry_count += 1;
    	    	regsvc(&id, SvcStatus::Readying);
    	    	pid = full_exec();
    	    	if pid == 0 {
    	    		eprintln!("{}Failed to restart service {}!", Red.paint(" * "), Red.paint(prompt.clone()));
    	    		continue;
    	    	}
    	    	regsvc(&id, SvcStatus::Running);
    	    }
    	}
    }
}
fn svc_stop(action_user: &User, env: &str, stop_way: &Value, svc_pid: pid_t) -> bool {
	match stop_way {
		Value::String(s) => {
			let p = asystem(&action_user, &s.replace("${PID}", &svc_pid.to_string()), env.clone());
			match p {
				Some(a) => {
					wait(a);
					return true;
				},
				None => {
					return false;
				},
			};
		},
		Value::Integer(i) => {
		    return send_signal(svc_pid, i.clone().try_into().unwrap_or(SIGTERM));
		},
		_ => {
			return false;
		},
	};
}
fn svc_exec(user: &User, env: &str, exec: &str, take_io: bool, pid_file: &Option<Value>) -> Option<pid_t> {
	let mut p = asystem(&user, exec.clone(), env.clone());
	if pid_file.is_some() {
		let pid_file = pid_file.as_ref().unwrap();
		if !pid_file.is_str() {
			eprintln!("{}Invalid value.", Red.paint(" * "));
			return svc_exec(user, exec, env, take_io, &None);
		}
		let pid_file = pid_file.clone();
		let pid_file = pid_file.as_str().unwrap();
		let mut pid = String::new();
		loop {
			if Path::new(pid_file.clone()).exists() {
				pid = fs::read_to_string(pid_file.clone()).unwrap();
				break;
			}
		}
		let pid = pid.parse::<pid_t>();
		p = match pid {
			Ok(a) => Some(a),
			Err(_) => {
				eprintln!("{}PID file format error!", Red.paint(" * "));
				return svc_exec(user, exec, env, take_io, &None);
			},
		};
	}
	//Abort take_io until airpnv instead of airup_su
	p
}
fn svcid_detect(svctomlpath: &str) -> String {
    if fs::symlink_metadata(svctomlpath.clone())
        .unwrap()
        .file_type()
        .is_symlink()
    {
        let rep = fs::read_link(svctomlpath.clone()).unwrap();
        return svcid_detect(&rep.to_string_lossy());
    } else {
        let n = &Path::new(svctomlpath)
            .file_name()
            .unwrap()
            .to_string_lossy();
        n.replace(".svc", &String::new())
    }
}
// End Service Supervisor
fn timer(dur: time::Duration) -> Result<Arc<RwLock<bool>>, Box<dyn std::error::Error>> {
	let thrd = Builder::new().name("timer".to_string());
	let val = Arc::new(RwLock::new(false));
	{
	    let val = val.clone();
	    thrd.spawn(move || {
		    sleep(dur);
		    (*val.write().unwrap()) = true;
	    })?;
	}
	Ok(val)
}
fn send_signal(pid: pid_t, sig: c_int) -> bool {
    unsafe {
	let rslt = kill(pid, sig);
	if rslt == 0 {
		return true;
	}
	false
	}
}
fn try_wait(pid: pid_t) -> Option<c_int> {
	unsafe {
		let mut status: c_int = 0;
		let wait_status = waitpid(pid, &mut status as *mut c_int, WNOHANG);
		if wait_status == 0 {
			return None;
		} else {
			return Some(status);
		}
	}
}
fn wait(pid: pid_t) -> c_int {
    unsafe {
    	let mut status: c_int = 0;
    	waitpid(pid, &mut status as *mut c_int, 0);
    	return status;
    }
}
fn stage_prestart_exec(dir: &str, paral: bool) {
    if !Path::new(dir.clone()).exists() {
        println!(
            "{}The specified prestart stage directory {} does not exist. Aborting...",
            Red.paint(" * "),
            dir
        );
        return;
    }
    let rd = fs::read_dir(dir.clone());
    if rd.is_err() {
        println!(
            "{}The specified prestart stage directory {} is not a directory. Aborting...",
            Red.paint(" * "),
            dir
        );
        return;
    }
    let rd = rd.unwrap();
    let rd: Vec<io::Result<fs::DirEntry>> = rd.collect();
    let mut rda: Vec<PathBuf> = Vec::new();
    for i in rd.iter() {
        if i.is_err() {
            continue;
        }
        rda.push(i.as_ref().unwrap().path());
    }
    let rd = rda;
    println!("{}Running PreStart Objects...", Green.paint(" * "));
    for i in rd.iter() {
        let child = system(&i.to_string_lossy());
        if child.is_none() {
            continue;
        }
        let child = child.unwrap();
        if !paral {
            wait(child);
        }
    }
}
fn system(cmd: &str) -> Option<pid_t> {
    let a = Command::new("sh").arg("-c").arg(cmd).spawn();
    match a {
        Ok(b) => Some(b.id().try_into().unwrap()),
        Err(_) => None,
    }
}
fn asystem(user: &User, cmd: &str, env_list: &str) -> Option<pid_t> {
    let mut command = String::from(env_list);
    command.push_str(" exec ");
    command.push_str(cmd);
    #[cfg(feature = "no_airupsu")]
    {
        system(&command)
    }
    #[cfg(not(feature = "no_airupsu"))]
    {
        let mode: &str = match user {
        	Id(_) => "--uid",
        	Name(_) => -u,
        };
        let user = user.to_string();
        let a = Command::new("airup_su")
            .arg(mode)
            .arg(user)
            .arg("-c")
            .arg(command)
            .spawn();
        match a {
            Ok(b) => Some(b.id().try_into().unwrap()),
            Err(_) => None,
        }
    }
}
fn get_toml_of(path: &str) -> Option<Value> {
    let fontext = fs::read_to_string(path);
    if fontext.is_err() {
        return None;
    }
    let fontext = fontext.unwrap();
    let rslt = fontext.parse::<Value>();
    if rslt.is_err() {
        return None;
    }
    Some(rslt.unwrap())
}
fn sigfillset_s(sset: &mut sigset_t) {
    unsafe {
        sigfillset(sset as *mut sigset_t);
    }
}
fn sigprocmask_s(a: i32, b: &mut sigset_t, c: &mut sigset_t) {
    unsafe {
        sigprocmask(a, b as *mut sigset_t, c as *mut sigset_t);
    }
}
fn new_sigset() -> sigset_t {
    unsafe { mem::zeroed() }
}
fn getpid_s() -> pid_t {
    unsafe { getpid() }
}
fn disable_signals() {
    let mut ssa = new_sigset();
    let mut ssb = new_sigset();
    sigfillset_s(&mut ssa);
    sigprocmask_s(SIG_BLOCK, &mut ssa, &mut ssb);
}
fn get_default_value(ns: &str, vid: &str) -> Option<Value> {
    let mut full_name = String::from(ns);
    full_name.push('/');
    full_name.push_str(vid);
    let rslt = (*DEFAULT_CFG).get(&full_name);
    if rslt.is_none() {
        return None;
    }
    Some(rslt.unwrap().clone())
}
fn tomlget(set: Option<&Value>, ns: &str, vid: &str) -> Option<Value> {
    if set.is_none() {
        return get_default_value(ns, vid);
    }
    let rslt = set.unwrap();
    let rslt = rslt.get(ns.clone());
    if rslt.is_none() {
        return get_default_value(ns, vid);
    }
    let rslt = rslt.unwrap();
    let rslt = rslt.get(vid.clone());
    if rslt.is_none() {
        return get_default_value(ns, vid);
    }
    Some(rslt.unwrap().clone())
}
fn g_airupconf(set: Option<&Value>, vid: &str) -> Value {
    let temp = tomlget(set, "airup", vid.clone());
    let default = Lazy::new(|| get_default_value("airup", vid).unwrap());
    if temp.is_none() {
        return default.clone();
    }
    let temp = temp.unwrap();
    if (!temp.is_str()) && (vid == "osname" || vid == "airup_home" || vid == "env_path") {
        return default.clone();
    } else if (!temp.is_bool()) && vid == "prestart_paral" {
        return default.clone();
    }
    let rslt = temp;
    rslt
}
fn g_milestonetoml(set: Option<&Value>, vid: &str) -> Option<Value> {
    let temp = tomlget(set, "milestone", vid.clone());
    let default = Lazy::new(|| get_default_value("milestone", vid));
    if temp.is_none() && vid != "prompt" && vid != "pre_exec" {
        return Some(default.as_ref().unwrap().clone());
    }
    if temp.is_none() && (vid == "prompt" || vid == "pre_exec") {
        return None;
    }
    let temp = temp.unwrap();
    if (!temp.is_str()) && (vid == "prompt" || vid == "description" || vid == "pre_exec") {
        return Some(default.as_ref().unwrap().clone());
    } else if (!temp.is_bool()) && vid == "paral" {
        return Some(default.as_ref().unwrap().clone());
    } else if (!temp.is_array()) && vid == "dependencies" {
        return Some(default.as_ref().unwrap().clone());
    }
    Some(temp)
}
fn pid_detect() {
    #[cfg(not(feature = "quickdbg"))]
    {
        if getpid_s() != 1 {
            println!("{}This program can only run as PID 1 as long as feature 'quickdbg' is not enabled.", Red.paint(" * "));
            exit(-1);
        }
        disable_signals();
    }
}
fn pathsetup(val: &str) {
    if val == "DONT_SETUP" {
        return;
    } else {
        env::set_var("PATH", val);
    }
}
fn get_milestone() -> String {
    let args: Vec<String> = env::args().collect();
    let mut milestone: String = String::new();
    for i in args.iter() {
        if i.starts_with("milestone=") {
            milestone = i.to_string()[10..].to_string();
        }
    }
    if milestone == String::new() {
        milestone = "default".to_string();
    }
    milestone
}
fn set_airenv(ms: &str, ad: &str, par: bool) {
    env::set_var("AIRUP_TARGET_MILESTONE", ms);
    env::set_var("AIRUP_HOME_DIR", ad);
    env::set_var("AIRUP_PARAL_PRESTART", par.to_string());
}
fn set_panic() {
    panic::set_hook(Box::new(|panic_info| ()));
}
// The following function is made for "dependencies" toml object resolving.
fn vv_to_vs(vv: Vec<Value>) -> Vec<String> {
    let mut vs = Vec::new();
    for i in vv {
        if !i.is_str() {
            continue;
        }
        let i = i.as_str().unwrap();
        vs.push(i.to_string());
    }
    vs
}
fn toml_env_setup(env_list: Value) {
    let env_list = env_list.as_table();
    let env_list = match env_list {
        Some(a) => a,
        None => {
            return;
        }
    };
    let keys: Vec<&String> = env_list.keys().collect();
    for key in keys {
        let value = env_list.get(key).unwrap();
        if !value.is_str() {
            continue;
        }
        let value = value.as_str().unwrap();
        env::set_var(key, value);
    }
}
fn milestone_dep(ad: &str, mdir: &str, deps: Vec<String>) {
    for i in deps.iter() {
        let mut dir = PathBuf::from(mdir.clone());
        dir.push(i);
        milestone_exec(ad, &dir.to_string_lossy());
    }
}
fn milestone_exec(ad: &str, dir: &str) {
    // Judge if the milestone exists
    if !Path::new(dir.clone()).exists() {
        println!(
            "{}The specified milestone {} does not exist.",
            Red.paint(" * "),
            Red.paint(dir.clone())
        );
        return;
    }
    // Find milestone.toml
    let mut mtpath = PathBuf::from(dir.clone());
    mtpath.push("milestone.toml");
    // Ready data
    let milestone_toml = get_toml_of(&mtpath.to_string_lossy());
    let invalid = std::ffi::OsString::from("invalid");
    let default_prompt = mtpath
        .parent()
        .unwrap()
        .file_name()
        .unwrap_or(&invalid)
        .to_string_lossy();
    let prompt = g_milestonetoml(milestone_toml.as_ref(), "prompt")
        .unwrap_or(Value::String(default_prompt.to_string()));
    let prompt = prompt.as_str().unwrap();
    let description = g_milestonetoml(milestone_toml.as_ref(), "description").unwrap();
    let description = description.as_str().unwrap();
    let paral = g_milestonetoml(milestone_toml.as_ref(), "paral")
        .unwrap()
        .as_bool()
        .unwrap();
    let env = g_milestonetoml(milestone_toml.as_ref(), "env_list").unwrap();
    // pre_exec may be Option::None.
    let pre_exec = g_milestonetoml(milestone_toml.as_ref(), "pre_exec");
    let dependencies = g_milestonetoml(milestone_toml.as_ref(), "dependencies").unwrap();
    let dependencies = dependencies.as_array().unwrap();
    let _files = fs::read_dir(dir.clone());
    if _files.is_err() {
        println!(
            "{}The specified milestone path {} is not a directory.",
            Red.paint(" * "),
            Red.paint(dir.clone())
        );
        return;
    }
    let _files: Vec<io::Result<fs::DirEntry>> = _files.unwrap().collect();
    let mut files: Vec<String> = Vec::new();
    for i in _files.iter() {
        files.push(i.as_ref().unwrap().path().to_string_lossy().to_string());
    }
    // Action
    toml_env_setup(env);
    println!(
        "{}Reaching milestone {}({})...",
        Green.paint(" * "),
        Green.paint(prompt.clone()),
        Purple.paint(description)
    );
    (*CURRENT_STAGE.write().unwrap()) = Stage::Milestones(prompt.to_string());
    match pre_exec {
        Some(a) => {
            if !a.is_str() {
                ()
            }
            let a = a.as_str().unwrap();
            system(a);
        }
        None => (),
    };
    milestone_dep(
        ad.clone(),
        &Path::new(dir.clone()).parent().unwrap().to_string_lossy(),
        vv_to_vs(dependencies.clone()),
    );
    if paral {
        milestone_svcexec(Box::leak(ad.to_string().into_boxed_str()), files);
    } else {
    }
}
fn milestone_svcexec(ad: &'static str, files: Vec<String>) {
    for i in files {
        if !i.ends_with(".svc") {
            continue;
        }
        svcrun(ad.clone(), Box::leak(i.into_boxed_str()));
    }
}
fn stage_milestone_start(ad: &str, dir: &str, milestone: &str) {
    let mut dir = PathBuf::from(dir);
    dir.push(milestone);
    milestone_exec(ad, &dir.to_string_lossy());
}
fn enable_rw() {
    let address = "tcp://127.0.0.1:61257";
    let server = Socket::new(Protocol::Rep0);
    let server = match server {
        Ok(a) => a,
        Err(b) => {
            eprintln!(
                "{}Failed to create NNG Socket({}): running in RO mode!",
                Red.paint(" * "),
                b
            );
            return;
        }
    };
    let c = server.listen(address.clone());
    match c {
        Ok(_) => (),
        Err(a) => {
            eprintln!(
                "{}Failed to listen address {}({}): running in RO mode!",
                Red.paint(" * "),
                address,
                a
            );
            return;
        }
    };
    let mut sups: HashMap<String, Socket> = HashMap::new();
    let supls = Socket::new(Protocol::Pull0);
    let supls = match supls {
        Ok(a) => a,
        Err(b) => {
            eprintln!(
                "{}Failed to create NNG Socket({}): running in RO mode!",
                Red.paint(" * "),
                b
            );
            return;
        }
    };
    let c = supls.listen("inproc://airup/regsvc");
    match c {
        Ok(_) => (),
        Err(a) => {
            eprintln!(
                "{}Failed to listen address {}({}): running in RO mode!",
                Red.paint(" * "),
                address,
                a
            );
            return;
        }
    };
    let base_dir = "inproc://airup/supervisors/";
    (*COMM_INIT.write().unwrap()) = true;
    println!("{}Starting communicating bridges...", Green.paint(" * "));
    loop {
        // Find new supervisors
        let msg = supls.try_recv();
        if msg.is_ok() {
            let msg = msg.unwrap();
            let mut mdir = String::from(base_dir.clone());
            let mut id = String::from_utf8_lossy(msg.as_slice()).to_string();
            let mut is_up = true;
            if id.starts_with("up ") {
                id = id[3..].to_string();
            } else if id.starts_with("down ") {
                id = id[5..].to_string();
                is_up = false;
            }
            if is_up {
                let skt = Socket::new(Protocol::Pair1);
                let skt = match skt {
                    Ok(a) => a,
                    Err(_) => {
                        continue;
                    }
                };
                mdir.push_str(&id);
                let c = skt.dial(&mdir);
                match c {
                    Ok(_) => (),
                    Err(_) => {
                        continue;
                    }
                };
                sups.insert(id, skt);
            } else {
                sups.get_mut(&id).unwrap().close();
                sups.remove(&id);
            }
        }
        // Detect IPC messages
        let msg = server.try_recv();
        if msg.is_ok() {
            let msg = msg.unwrap();
            let msg = String::from_utf8_lossy(msg.as_slice());
            if msg.starts_with("svc ") {
            	let msg = &msg[4..];
            	if msg.starts_with("start ") {
            		let msg = &msg[6..];
            	} else if msg.starts_with("stop ") {
            		let msg = &msg[5..];
            		let guard = false;
            		if guard {
            			unimplemented!();
            		} else {
            			let sp = sups.get(msg.clone());
            			match sp {
            				Some(a) => match a.send("down".as_bytes()) {
            						Ok(_) => (),
            						Err(_) => (),
            					},
            				None => match server.send("SvcNotRunning".as_bytes()) {
            						    Ok(_) => (),
            						    Err(_) => (),
            					    },
            			};
            		}
            	} else if msg.starts_with("restart ") {
            		let msg = &msg[8..];
            	} else if msg.starts_with("status ") {
            		let msg = &msg[7..];
            		let sp = sups.get(msg.clone());
            		match sp {
            			Some(a) => {
            				let status = svc_running_core(msg);
            				let status_str = match status {
            					SvcStatus::Readying => "Readying",
            					SvcStatus::Running => "Running",
            					SvcStatus::Working => "Working",
            					SvcStatus::Stopped => "Stopped",
            					SvcStatus::Unmentioned => "SvcNotRunning",
            				};
            				let pid = match status {
            					SvcStatus::Running => {
            						match a.send("pid".as_bytes()) {
            							Ok(_) => {
            								match a.recv() {
            									Ok(msg) => {
            										String::from_utf8_lossy(msg.as_slice()).to_string()
            									},
            									Err(_) => "0".to_string(),
            								}
            							},
            							Err(_) => "0".to_string(),
            						}
            					},
            					_ => "0".to_string(),
            				};
            				let mut newmsg = String::from(status_str);
            				newmsg.push(' ');
            				newmsg.push_str(&pid);
            				match server.send(newmsg.as_bytes()) {
            					Ok(_) => (),
            					Err(_) => {
            						continue;
            					},
            				};
            			},
            			None => {
            				match server.send("SvcNotRunning".as_bytes()) {
            					Ok(_) => (),
            					Err(_) => {
            						continue;
            					},
            				};
            			},
            		};
            	}
            } else if msg.starts_with("system ") {
            	let msg = &msg[7..];
            }
        }
    }
}
fn main() {
    pid_detect();
    set_panic();
    let airup_conf = get_toml_of(AIRUP_CONF);
    //Prepare some values from airup.conf
    let osname = g_airupconf(airup_conf.as_ref(), "osname");
    let osname = osname.as_str().unwrap();
    pathsetup(
        g_airupconf(airup_conf.as_ref(), "env_path")
            .as_str()
            .unwrap(),
    );
    println!(
        "{} {} is launching {}...",
        Purple.paint("Airup"),
        Purple.paint(AIRUP_VERSION.clone()),
        Green.paint(osname)
    );
    let milestone = get_milestone();
    let airup_home = g_airupconf(airup_conf.as_ref(), "airup_home");
    let airup_home = airup_home.as_str().unwrap();
    let prestart_paral = g_airupconf(airup_conf.as_ref(), "prestart_paral");
    let prestart_paral = prestart_paral.as_bool().unwrap();
    set_airenv(&milestone, airup_home.clone(), prestart_paral);
    let mut prestart_dir = PathBuf::from(airup_home.clone());
    prestart_dir.push("prestart");
    stage_prestart_exec(
        prestart_dir.to_str().unwrap_or(
            get_default_value("internal", "prestart_dir")
                .unwrap()
                .as_str()
                .unwrap(),
        ),
        prestart_paral,
    );
    let thrd = Builder::new().name("ipcmgr".to_string());
    let rwmode = thrd.spawn(|| enable_rw()).unwrap();
    loop {
        if (*COMM_INIT.read().unwrap()) == true {
            break;
        }
    }
    let mut milestones_dir = PathBuf::from(airup_home.clone());
    milestones_dir.push("milestones");
    let milestones_dir = milestones_dir.to_string_lossy();
    stage_milestone_start(airup_home.clone(), &milestones_dir, &milestone);
    rwmode.join().unwrap();
    loop {}
}
