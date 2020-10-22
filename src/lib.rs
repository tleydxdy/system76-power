#[macro_use]
extern crate err_derive;
extern crate intel_pstate as pstate;
#[macro_use]
extern crate log;
#[macro_use]
extern crate async_trait;

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

#[async_trait]
pub trait Power {
    async fn performance(&self) -> Result<(), String>;
    async fn balanced(&self) -> Result<(), String>;
    async fn battery(&self) -> Result<(), String>;
    async fn get_graphics(&self) -> Result<String, String>;
    async fn get_profile(&self) -> Result<String, String>;
    async fn get_switchable(&self) -> Result<bool, String>;
    async fn set_graphics(&self, vendor: &str) -> Result<(), String>;
    async fn get_graphics_power(&self) -> Result<bool, String>;
    async fn set_graphics_power(&self, power: bool) -> Result<(), String>;
    async fn auto_graphics_power(&self) -> Result<(), String>;
}

// Helper function for errors
pub(crate) fn err_str<E: ::std::fmt::Display>(err: E) -> String { format!("{}", err) }
