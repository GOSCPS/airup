use ansi_term::Color::*;
use async_std::{fs, prelude::*, println, sync::RwLock, task, io, path::PathBuf};
use libc::{getpid, pid_t, sigfillset, sigprocmask, sigset_t, uid_t, SIG_BLOCK};
use once_cell::sync::Lazy;
use std::{
    collections::HashMap,
    env,
    fmt::Display,
    fmt::Formatter,
    mem, panic,
    process::exit,
    process::{Child, Command},
};
use toml::Value;

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
enum SvcStatus {
    Readying,
    Running,
    Working,
    Stopped,
    Unmentioned,
}

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
    a
});

#[cfg(not(feature = "quickdbg"))]
static AIRUP_CONF: &str = "/etc/airup.conf";

#[cfg(feature = "quickdbg")]
static AIRUP_CONF: &str = "debug/airup.conf";

async fn stage_prestart_exec(dir: &str, paral: bool) {
    if !std::path::Path::new(dir.clone()).exists() {
        println!(
            "{}The specified prestart stage directory {} does not exist. Aborting...",
            Red.paint(" * "),
            dir
        )
        .await;
        return;
    }
    let rd = fs::read_dir(dir.clone()).await;
    if rd.is_err() {
        println!(
            "{}The specified prestart stage directory {} is not a directory. Aborting...",
            Red.paint(" * "),
            dir
        )
        .await;
        return;
    }
    let rd = rd.unwrap();
    let rd: Vec<io::Result<fs::DirEntry>> = rd.collect().await;
    let mut rda: Vec<PathBuf> = Vec::new();
    for i in rd.iter() {
    	if i.is_err() {
    		continue;
    	}
    	rda.push(i.as_ref().unwrap().path());
    }
    let rd = rda;
    for i in rd.iter() {
        let mut child = system(i.to_str().unwrap()).await.unwrap();
        if !paral {
            child.wait();
        }
    }
}
async fn system(cmd: &str) -> Option<Child> {
        let a = Command::new("sh").arg("-c").arg(cmd).spawn();
        match a {
            Ok(b) => Some(b),
            Err(_) => None,
        }
}
async fn asystem(user: &User, cmd: &str, env_list: &str) -> Option<Child> {
    let mut command = String::from(env_list);
    command.push_str(" exec ");
    command.push_str(cmd);
    #[cfg(feature = "no_airupsu")]
    {
        system(&command).await
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
async fn get_toml_of(path: &str) -> Option<Value> {
    let fontext = fs::read_to_string(path).await;
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
fn g_airupconf(set: Option<Value>, vid: &str) -> Option<Value> {
    let temp = tomlget(set, "airup", vid.clone());
    if temp.is_none() {
        return get_default_value("airup", vid);
    }
    let temp = temp.unwrap();
    if (!temp.is_str()) && (vid == "osname" || vid == "airup_home" || vid == "env_path") {
        return get_default_value("airup", vid);
    } else if (!temp.is_bool()) && vid == "prestart_paral" {
        return get_default_value("airup", vid);
    }
    let rslt = temp;
    Some(rslt)
}
async fn pid_detect() {
    #[cfg(not(feature = "quickdbg"))]
    {
        if getpid_s() != 1 {
            println!("{}This program can only run as PID 1 as long as feature 'quickdbg' is not enabled.", Red.paint(" * ")).await;
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
        std::println!("[!!!]Airup Panic");
    }));
}
#[async_std::main]
async fn main() {
    pid_detect().await;
    set_panic();
    let airup_conf = get_toml_of(AIRUP_CONF).await;
    //Prepare some values from airup.conf
    let osname = g_airupconf(airup_conf.clone(), "osname").unwrap();
    let osname = osname.as_str().unwrap();
    pathsetup(
        g_airupconf(airup_conf.clone(), "env_path")
            .unwrap()
            .as_str()
            .unwrap(),
    );
    println!(
        "{} {} is launching {}...",
        Purple.paint("Airup"),
        Purple.paint(AIRUP_VERSION.clone()),
        Green.paint(osname)
    )
    .await;
    let milestone = get_milestone();
    let airup_home = g_airupconf(airup_conf.clone(), "airup_home").unwrap();
    let airup_home = airup_home.as_str().unwrap();
    let prestart_paral = g_airupconf(airup_conf, "prestart_paral").unwrap();
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
    )
    .await;
}
