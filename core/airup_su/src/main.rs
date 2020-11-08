use clap::{App, Arg};
use users::get_user_by_name;
use libc::setuid;
use nix::unistd::execvp;
use std::ffi::CString;

fn main() {
    let matches = App::new("Airup Security User Util")
        .version("9999")
        .about("Use a specificed user for spawning Airup Service")
        .arg(
        	Arg::with_name("u")
        	.short("u")
        	.value_name("USER")
        	.help("Set user to spawn")
        	.takes_value(true)
        )
        .arg(
        	Arg::with_name("c")
        	.short("c")
        	.value_name("CMD")
        	.help("Specific command to execute")
        	.required(true)
        	.takes_value(true)
        )
        .get_matches();
    let user = matches.value_of("u").unwrap_or("default");
    let cmd = matches.value_of("c").unwrap();
    if user != "default" {
        let uid = get_user_by_name(user).unwrap().uid();
        unsafe {
            setuid(uid);
        }
    }
    let a = CString::new("sh").unwrap();
    let b = CString::new("-c").unwrap();
    let c = CString::new(cmd).unwrap();
    let args = vec![a, b, c];
    execvp(&CString::new("/bin/sh").unwrap(), &args).unwrap();
}
