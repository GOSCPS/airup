use clap::{App, Arg};
use std::{process::Command, os::unix::process::CommandExt};
use users::get_user_by_name;
use libc::uid_t;

fn main() {
    let matches = App::new("Airup Su Improved")
        .version("n/a")
        .about("To exec a program with specified user.")
        .arg(
            Arg::with_name("user")
                .short("u")
                .value_name("USERNAME")
                .help("Specify a username, and find UID automatically.")
                .conflicts_with("uid")
                .takes_value(true)
        )
        .arg(
        	Arg::with_name("uid")
        	    .long("uid")
        	    .value_name("UID")
        	    .help("Specify a UID.")
        	    .conflicts_with("user")
        	    .takes_value(true)
        )
        .arg(
        	Arg::with_name("cmd")
        	    .short("c")
        	    .value_name("COMMAND")
        	    .help("Specify the command to run.")
        	    .takes_value(true)
        	    .required(true)
        )
        .get_matches();
    let uid: uid_t;
    let cmd = matches.value_of("cmd").unwrap();
    if matches.is_present("user") {
    	uid = get_user_by_name(matches.value_of("user").unwrap()).unwrap().uid();
    } else if matches.is_present("uid") {
    	uid = matches.value_of("uid").unwrap().parse::<uid_t>().unwrap_or(users::get_current_uid());
    } else {
    	uid = users::get_current_uid();
    }
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .uid(uid)
        .exec();
}
