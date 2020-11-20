use ansi_term::Color::*;
use libc::{getpid, pid_t, sigfillset, sigprocmask, sigset_t, uid_t, SIG_BLOCK};
use nng::{Protocol, Socket};
use once_cell::sync::Lazy;
use std::{
    cmp::PartialEq,
    collections::HashMap,
    env,
    fmt::Display,
    fmt::Formatter,
    fs, io, mem, panic,
    path::{Path, PathBuf},
    process::{exit, Child, Command},
    sync::RwLock,
    thread::Builder,
};
use toml::{map::Map, Value};

enum User {
    Id(uid_t),
    Name(String),
}
impl User {
    fn is_id(&self) -> bool {
        match self {
            User::Id(_) => true,
            User::Name(_) => false,
        }
    }
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
    if svc_running_core(id.clone()) == SvcStatus::Readying {
        loop {
            if svc_running_core(id) != SvcStatus::Readying {
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
fn delsvc(id: &str) {
    (*SVC_STATUS.write().unwrap()).remove(id);
}
fn svcrun(airup_dir: &'static str, svctomlpath: &'static str) -> bool {
    let svctoml = get_toml_of(svctomlpath);
    if svctoml.is_none() {
        println!(
            "{}Some problems happened when launching service {}.",
            Red.paint(" * "),
            svctomlpath
        );
        return false;
    }
    let id = svcid_detect(svctomlpath.clone());
    if svc_running_block(&id) {
        return true;
    }
    let thrd = Builder::new().name(id.to_string());
    let svctoml = svctoml.unwrap();
    let thrd = thrd.spawn(move || svc_supervisor_main(&id, airup_dir, svctoml));
    if thrd.is_err() {
        println!("{}OS Error: Failed to create thread!", Red.paint(" *** "));
        return false;
    }
    true
}
fn svc_dep(airup_dir: &'static str, deps: Vec<String>) {
    for i in deps {
        let mut i = String::from(i);
        if i.starts_with("alias::") {
            let e = PathBuf::from(airup_dir.clone());
            unreachable!();
        } else {
            i.push_str(".svc");
            svcrun(airup_dir.clone(), Box::leak(i.into_boxed_str()));
        }
    }
}
fn g_svc(set: Value, vid: &str) -> Option<Value> {
    let temp = tomlget(Some(set), "svc", vid.clone());
    let default = Lazy::new(|| get_default_value("svc", vid));
    if temp.is_none() && vid != "prompt" {
        return Some(default.as_ref().unwrap().clone());
    }
    if temp.is_none() && vid == "prompt" {
    	return None;
    }
    let temp = temp.unwrap();
    Some(temp)
}
fn svc_supervisor_main(id: &str, airup_dir: &str, svctoml: Value) {
    regsvc(id.clone(), SvcStatus::Readying);
    let prompt = g_svc(svctoml.clone(), "prompt")
        .unwrap_or(Value::String(id.to_string()))
        .as_str()
        .unwrap_or(id.clone());
    let desc = g_svc(svctoml.clone(), "description")
        .unwrap()
        .as_str()
        .unwrap();
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
        let mut child = child.unwrap();
        if !paral {
            let _ = child.wait();
        }
    }
}
fn system(cmd: &str) -> Option<Child> {
    let a = Command::new("sh").arg("-c").arg(cmd).spawn();
    match a {
        Ok(b) => Some(b),
        Err(_) => None,
    }
}
fn asystem(user: &User, cmd: &str, env_list: &str) -> Option<Child> {
    let mut command = String::from(env_list);
    command.push_str(" exec ");
    command.push_str(cmd);
    #[cfg(feature = "no_airupsu")]
    {
        system(&command)
    }
    #[cfg(not(feature = "no_airupsu"))]
    {
        let mode: &str;
        if user.is_id() {
            mode = "--uid";
        } else {
            mode = "-u";
        }
        let user = user.to_string();
        let a = Command::new("airup_su")
            .arg(mode)
            .arg(user)
            .arg("-c")
            .arg(command)
            .spawn();
        match a {
            Ok(b) => Some(b),
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
fn tomlget(set: Option<Value>, ns: &str, vid: &str) -> Option<Value> {
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
fn g_airupconf(set: Option<Value>, vid: &str) -> Value {
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
fn g_milestonetoml(set: Option<Value>, vid: &str) -> Option<Value> {
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
    panic::set_hook(Box::new(|panic_info| {
        println!("[!!!]Airup Panic");
        eprintln!("Error Message: {}", panic_info);
        loop {}
    }));
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
    let default_prompt = mtpath.parent().unwrap().file_name().unwrap_or(&invalid).to_string_lossy();
    let prompt = g_milestonetoml(milestone_toml.clone(), "prompt")
        .unwrap_or(Value::String(default_prompt.to_string()));
    let prompt = prompt.as_str().unwrap();
    let description = g_milestonetoml(milestone_toml.clone(), "description").unwrap();
    let description = description.as_str().unwrap();
    let paral = g_milestonetoml(milestone_toml.clone(), "paral")
        .unwrap()
        .as_bool()
        .unwrap();
    let env = g_milestonetoml(milestone_toml.clone(), "env_list").unwrap();
    // pre_exec may be Option::None.
    let pre_exec = g_milestonetoml(milestone_toml.clone(), "pre_exec");
    let dependencies = g_milestonetoml(milestone_toml, "dependencies").unwrap();
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
    let server = Socket::new(Protocol::Pair1);
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
    loop {
        // Find new supervisors
        let msg = supls.try_recv();
        if msg.is_ok() {
            let msg = msg.unwrap();
            let skt = Socket::new(Protocol::Pair1);
            let skt = match skt {
                Ok(a) => a,
                Err(_) => { continue; }
            };
            let mut mdir = String::from(base_dir.clone());
            mdir.push_str(&String::from_utf8_lossy(msg.as_slice()));
            let c = skt.dial(&mdir);
            match c {
                Ok(_) => (),
                Err(_) => { continue; }
            };
            sups.insert(mdir, skt);
        }
        // Detect IPC messages
        let msg = server.try_recv();
    }
}
fn main() {
    pid_detect();
    set_panic();
    let airup_conf = get_toml_of(AIRUP_CONF);
    //Prepare some values from airup.conf
    let osname = g_airupconf(airup_conf.clone(), "osname");
    let osname = osname.as_str().unwrap();
    pathsetup(
        g_airupconf(airup_conf.clone(), "env_path")
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
    let airup_home = g_airupconf(airup_conf.clone(), "airup_home");
    let airup_home = airup_home.as_str().unwrap();
    let prestart_paral = g_airupconf(airup_conf, "prestart_paral");
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
    let mut milestones_dir = PathBuf::from(airup_home.clone());
    milestones_dir.push("milestones");
    let milestones_dir = milestones_dir.to_string_lossy();
    stage_milestone_start(airup_home.clone(), &milestones_dir, &milestone);
    let jh = rwmode.join();
    if jh.is_err() {
        println!(
            "{}Failed to use high-performance mode: using loops!",
            Red.paint(" * ")
        );
    }
    loop {}
}
