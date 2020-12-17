#[cfg(target_os = "linux")]
use libc::{reboot, LINUX_REBOOT_CMD_POWER_OFF, LINUX_REBOOT_CMD_RESTART};
use std::process::exit;
#[cfg(target_os = "linux")]
pub fn poweroff() {
    reboot(LINUX_REBOOT_CMD_POWER_OFF);
}
#[cfg(target_os = "linux")]
pub fn restart() {
    reboot(LINUX_REBOOT_CMD_POWER_OFF);
}
#[cfg(not(target_os = "linux"))]
pub fn poweroff() {
	exit(-1);
}
#[cfg(not(target_os = "linux"))]
pub fn restart() {
	exit(0);
}
