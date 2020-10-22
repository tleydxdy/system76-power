use crate::{err_str, Power, DBUS_IFACE, DBUS_NAME, DBUS_PATH};
use clap::ArgMatches;
use dbus::{
    arg::{AppendAll, ReadAll},
    nonblock::{Proxy, SyncConnection},
};
use dbus_tokio::connection;
use pstate::PState;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use sysfs_class::{Backlight, Brightness, Leds, SysClass};

static TIMEOUT: u64 = 60 * 1000;

pub struct PowerClient {
    bus: Arc<SyncConnection>,
}

impl PowerClient {
    pub fn new() -> Result<PowerClient, String> {
        let (resource, bus) = connection::new_session_sync().map_err(err_str)?;

        tokio::spawn(async {
            let err = resource.await;
            panic!("Lost connection to D-Bus: {}", err);
        });

        Ok(PowerClient { bus })
    }

    async fn call_method<A: AppendAll, R: ReadAll + 'static>(
        &self,
        method: &str,
        args: A,
    ) -> Result<R, String> {
        let proxy = Proxy::new(DBUS_NAME, DBUS_PATH, Duration::from_millis(TIMEOUT), self.bus.as_ref());

        proxy.method_call::<R, A, _, _>(DBUS_IFACE, method, args).await
             .map_err(|why| format!("daemon returned an error message: {}", err_str(why)))
    }

    async fn set_profile(&self, profile: &str) -> Result<(), String> {
        println!("setting power profile to {}", profile);
        self.call_method::<(), ()>(profile, ()).await
    }
}

#[async_trait]
impl Power for PowerClient {
    async fn performance(&self) -> Result<(), String> {
        self.set_profile("Performance").await
    }

    async fn balanced(&self) -> Result<(), String> {
        self.set_profile("Balanced").await
    }

    async fn battery(&self) -> Result<(), String> {
        self.set_profile("Battery").await
    }

    async fn get_graphics(&self) -> Result<String, String> {
        let res: (String,) = self.call_method("GetGraphics", ()).await?;
        Ok(res.0)
    }

    async fn get_profile(&self) -> Result<String, String> {
        let res: (String,) = self.call_method("GetProfile", ()).await?;
        Ok(res.0)
    }

    async fn get_switchable(&self) -> Result<bool, String> {
        let res: (bool,) = self.call_method("GetSwitchable", ()).await?;
        Ok(res.0)
    }

    async fn set_graphics(&self, vendor: &str) -> Result<(), String> {
        println!("setting graphics to {}", vendor);
        self.call_method::<_, ()>("SetGraphics", (vendor,)).await
    }

    async fn get_graphics_power(&self) -> Result<bool, String> {
        let res: (bool,) = self.call_method("GetGraphicsPower", ()).await?;
        Ok(res.0)
    }

    async fn set_graphics_power(&self, power: bool) -> Result<(), String> {
        println!("turning discrete graphics {}", if power { "on" } else { "off" });
        self.call_method::<_, ()>("SetGraphicsPower", (power,)).await
    }

    async fn auto_graphics_power(&self) -> Result<(), String> {
        println!("setting discrete graphics to turn off when not in use");
        self.call_method::<(), ()>("AutoGraphicsPower", ()).await
    }
}

async fn profile(client: &mut PowerClient) -> io::Result<()> {
    let profile = client.get_profile().await.ok();
    let profile = profile.as_ref().map_or("?", |s| s.as_str());
    println!("Power Profile: {}", profile);

    if let Ok(values) = PState::new().and_then(|pstate| pstate.values()) {
        println!(
            "CPU: {}% - {}%, {}",
            values.min_perf_pct,
            values.max_perf_pct,
            if values.no_turbo { "No Turbo" } else { "Turbo" }
        );
    }

    for backlight in Backlight::iter() {
        let backlight = backlight?;
        let brightness = backlight.actual_brightness()?;
        let max_brightness = backlight.max_brightness()?;
        let ratio = (brightness as f64) / (max_brightness as f64);
        let percent = (ratio * 100.0) as u64;
        println!("Backlight {}: {}/{} = {}%", backlight.id(), brightness, max_brightness, percent);
    }

    for backlight in Leds::iter_keyboards() {
        let backlight = backlight?;
        let brightness = backlight.brightness()?;
        let max_brightness = backlight.max_brightness()?;
        let ratio = (brightness as f64) / (max_brightness as f64);
        let percent = (ratio * 100.0) as u64;
        println!(
            "Keyboard Backlight {}: {}/{} = {}%",
            backlight.id(),
            brightness,
            max_brightness,
            percent
        );
    }

    Ok(())
}

#[tokio::main]
pub async fn client(subcommand: &str, matches: &ArgMatches) -> Result<(), String> {
    let mut client = PowerClient::new()?;

    match subcommand {
        "profile" => match matches.value_of("profile") {
            Some("balanced") => client.balanced().await,
            Some("battery") => client.battery().await,
            Some("performance") => client.performance().await,
            _ => profile(&mut client).await.map_err(err_str),
        },
        "graphics" => match matches.subcommand() {
            ("compute", _) => client.set_graphics("compute").await,
            ("hybrid", _) => client.set_graphics("hybrid").await,
            ("integrated", _) | ("intel", _) => client.set_graphics("integrated").await,
            ("nvidia", _) => client.set_graphics("nvidia").await,
            ("switchable", _) => {
                if client.get_switchable().await? {
                    println!("switchable");
                } else {
                    println!("not switchable");
                }
                Ok(())
            }
            ("power", Some(matches)) => match matches.value_of("state") {
                Some("auto") => client.auto_graphics_power().await,
                Some("off") => client.set_graphics_power(false).await,
                Some("on") => client.set_graphics_power(true).await,
                _ => {
                    if client.get_graphics_power().await? {
                        println!("on (discrete)");
                    } else {
                        println!("off (discrete)");
                    }
                    Ok(())
                }
            },
            _ => {
                println!("{}", client.get_graphics().await?);
                Ok(())
            }
        },
        _ => Err(format!("unknown sub-command {}", subcommand)),
    }
}
