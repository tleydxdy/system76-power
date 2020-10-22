#[macro_use]
extern crate err_derive;
extern crate intel_pstate as pstate;
#[macro_use]
extern crate log;

pub mod client;
pub mod daemon;
pub mod disks;
pub mod errors;
pub mod fan;
pub mod graphics;
pub mod hid_backlight;
pub mod hotplug;
pub mod kernel_parameters;
pub mod logging;
pub mod modprobe;
pub mod module;
pub mod mux;
pub mod pci;
pub mod radeon;
pub mod sideband;
pub mod snd;
pub mod util;
pub mod wifi;

include!(concat!(env!("OUT_DIR"), "/version.rs"));

pub static DBUS_NAME: &'static str = "com.system76.PowerDaemon";
pub static DBUS_PATH: &'static str = "/com/system76/PowerDaemon";
pub static DBUS_IFACE: &'static str = "com.system76.PowerDaemon";

pub trait Power {
    fn performance(&self) -> Result<(), String>;
    fn balanced(&self) -> Result<(), String>;
    fn battery(&self) -> Result<(), String>;
    fn get_graphics(&self) -> Result<String, String>;
    fn get_profile(&self) -> Result<String, String>;
    fn get_switchable(&self) -> Result<bool, String>;
    fn set_graphics(&self, vendor: &str) -> Result<(), String>;
    fn get_graphics_power(&self) -> Result<bool, String>;
    fn set_graphics_power(&self, power: bool) -> Result<(), String>;
    fn auto_graphics_power(&self) -> Result<(), String>;
}

// Helper function for errors
pub(crate) fn err_str<E: ::std::fmt::Display>(err: E) -> String { format!("{}", err) }
