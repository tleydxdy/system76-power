use dbus::{
    arg,
    channel::{
        MatchingReceiver,
        Sender,
    },
    nonblock::SyncConnection,
    message::{
        MatchRule,
        Message,
    },
};
use dbus_crossroads::{
    Crossroads,
    IfaceBuilder,
    MethodErr,
};
use dbus_tokio::connection;
use std::{
    boxed::Box,
    fmt::Debug,
    fs,
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
        Mutex,
    },
    pin::Pin,
    time::Duration,
    thread,
};
use tokio::{
    signal::unix::{signal, SignalKind},
    stream::StreamExt,
    time::delay_for,
};

use crate::{
    err_str,
    errors::ProfileError,
    fan::FanDaemon,
    graphics::Graphics,
    hid_backlight,
    hotplug::HotPlugDetect,
    kernel_parameters::{KernelParameter, NmiWatchdog},
    mux::DisplayPortMux,
    Power, DBUS_IFACE, DBUS_NAME, DBUS_PATH,
};

mod profiles;

use self::profiles::*;

static CONTINUE: AtomicBool = AtomicBool::new(true);

fn signal_handling() {
    let int = signal(SignalKind::interrupt()).unwrap().map(|_| "SIGINT");
    let hup = signal(SignalKind::hangup()).unwrap().map(|_| "SIGHUP");
    let term = signal(SignalKind::terminate()).unwrap().map(|_| "SIGTERM");
    let mut signals = int.merge(hup).merge(term);

    tokio::spawn(async move {
        while let Some(sig) = signals.next().await {
            info!("caught signal: {}", sig);
            CONTINUE.store(false, Ordering::SeqCst);
        }
    });
}

// Disabled by default because some systems have quirky ACPI tables that fail to resume from
// suspension.
static PCI_RUNTIME_PM: AtomicBool = AtomicBool::new(false);

// TODO: Whitelist system76 hardware that's known to work with this setting.
fn pci_runtime_pm_support() -> bool { PCI_RUNTIME_PM.load(Ordering::SeqCst) }

struct PowerDaemon {
    initial_set:         bool,
    graphics:            Graphics,
    power_profile:       Mutex<String>,
    dbus_connection:     Arc<SyncConnection>,
}

impl PowerDaemon {
    fn new(
        dbus_connection: Arc<SyncConnection>,
    ) -> Result<PowerDaemon, String> {
        let graphics = Graphics::new().map_err(err_str)?;
        Ok(PowerDaemon {
            initial_set: false,
            graphics,
            power_profile: Mutex::new(String::new()),
            dbus_connection,
        })
    }

    fn apply_profile(
        &self,
        func: fn(&mut Vec<ProfileError>, bool),
        name: &str,
    ) -> Result<(), String> {
        let mut power_profile = self.power_profile.lock().unwrap();

        if *power_profile == name {
            info!("profile was already set");
            return Ok(())
        }

        let mut profile_errors = Vec::new();

        func(&mut profile_errors, self.initial_set);

        let message =
            Message::new_signal(DBUS_PATH, DBUS_NAME, "PowerProfileSwitch").unwrap().append1(name);

        if let Err(()) = self.dbus_connection.send(message) {
            error!("failed to send power profile switch message");
        }

        *power_profile = name.into();

        if profile_errors.is_empty() {
            Ok(())
        } else {
            let mut error_message = String::from("Errors found when setting profile:");
            for error in profile_errors.drain(..) {
                error_message = format!("{}\n    - {}", error_message, error);
            }

            Err(error_message)
        }
    }

    async fn test(&self) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait]
impl Power for PowerDaemon {
    async fn battery(&self) -> Result<(), String> {
        self.apply_profile(battery, "Battery").map_err(err_str)
    }

    async fn balanced(&self) -> Result<(), String> {
        self.apply_profile(balanced, "Balanced").map_err(err_str)
    }

    async fn performance(&self) -> Result<(), String> {
        self.apply_profile(performance, "Performance").map_err(err_str)
    }

    async fn get_graphics(&self) -> Result<String, String> {
        self.graphics.get_vendor().map_err(err_str)
    }

    async fn get_profile(&self) -> Result<String, String> { Ok(self.power_profile.lock().unwrap().clone()) }

    async fn get_switchable(&self) -> Result<bool, String> { Ok(self.graphics.can_switch()) }

    async fn set_graphics(&self, vendor: &str) -> Result<(), String> {
        self.graphics.set_vendor(vendor).map_err(err_str)
    }

    async fn get_graphics_power(&self) -> Result<bool, String> {
        self.graphics.get_power().map_err(err_str)
    }

    async fn set_graphics_power(&self, power: bool) -> Result<(), String> {
        self.graphics.set_power(power).map_err(err_str)
    }

    async fn auto_graphics_power(&self) -> Result<(), String> {
        self.graphics.auto_power().map_err(err_str)
    }
}

#[tokio::main]
pub async fn daemon() -> Result<(), String> {
    signal_handling();
    let pci_runtime_pm = std::env::var("S76_POWER_PCI_RUNTIME_PM").ok().map_or(false, |v| v == "1");

    info!(
        "Starting daemon{}",
        if pci_runtime_pm { " with pci runtime pm support enabled" } else { "" }
    );
    PCI_RUNTIME_PM.store(pci_runtime_pm, Ordering::SeqCst);

    info!("Connecting to dbus system bus");
    let (resource, c) = connection::new_system_sync().map_err(err_str)?;

    tokio::spawn(async {
        let err = resource.await;
        panic!("Lost connection to D-Bus: {}", err);
    });

    let mut daemon = PowerDaemon::new(c.clone())?;
    let nvidia_exists = !daemon.graphics.nvidia.is_empty();

    info!("Disabling NMI Watchdog (for kernel debugging only)");
    NmiWatchdog::default().set(b"0");

    // Get the NVIDIA device ID before potentially removing it.
    let nvidia_device_id = if nvidia_exists {
        fs::read_to_string("/sys/bus/pci/devices/0000:01:00.0/device").ok()
    } else {
        None
    };

    info!("Setting automatic graphics power");
    match daemon.auto_graphics_power().await {
        Ok(()) => (),
        Err(err) => {
            warn!("Failed to set automatic graphics power: {}", err);
        }
    }

    info!("Initializing with the balanced profile");
    if let Err(why) = daemon.balanced().await {
        warn!("Failed to set initial profile: {}", why);
    }
    daemon.initial_set = true;

    info!("Registering dbus name {}", DBUS_NAME);
    c.request_name(DBUS_NAME, false, true, false).await.map_err(err_str)?;

    info!("Adding dbus path {} with interface {}", DBUS_PATH, DBUS_IFACE);
    let mut cr = Crossroads::new();
    cr.set_async_support(Some((c.clone(), Box::new(|x| { tokio::spawn(x); }))));
    let iface_token = cr.register(DBUS_IFACE, |b| {
        sync_action_method(b, "Test", PowerDaemon::test);
        sync_action_method(b, "Performance", PowerDaemon::performance);
        sync_action_method(b, "Balanced", PowerDaemon::balanced);
        sync_action_method(b, "Battery", PowerDaemon::battery);
        sync_get_method(b, "GetGraphics", "vendor", PowerDaemon::get_graphics);
        // XXX
        //sync_set_method(b, "SetGraphics", "vendor", |d, s: String| d.set_graphics(&s));
        sync_get_method(b, "GetProfile", "profile", PowerDaemon::get_profile);
        sync_get_method(b, "GetSwitchable", "switchable", PowerDaemon::get_switchable);
        sync_get_method(b, "GetGraphicsPower", "power", PowerDaemon::get_graphics_power);
        sync_set_method(b, "SetGraphicsPower", "power", PowerDaemon::set_graphics_power);
        b.signal::<(u64,), _>("HotPlugDetect", ("port",));
        b.signal::<(&str,), _>("PowerProfileSwitch", ("profile",));
    });
    cr.insert(DBUS_PATH, &[iface_token], daemon);

    let cr = Arc::new(std::sync::Mutex::new(cr));
    c.start_receive(MatchRule::new_method_call(), Box::new(move |msg, c| {
        cr.lock().unwrap().handle_message(msg, c).unwrap();
        true
    }));
         
    // Spawn hid backlight daemon
    let _hid_backlight = thread::spawn(|| hid_backlight::daemon());

    let mut fan_daemon = FanDaemon::new(nvidia_exists);

    let hpd_res = unsafe { HotPlugDetect::new(nvidia_device_id) };

    let mux_res = unsafe { DisplayPortMux::new() };

    let hpd = || -> [bool; 4] {
        if let Ok(ref hpd) = hpd_res {
            unsafe { hpd.detect() }
        } else {
            [false; 4]
        }
    };

    let mut last = hpd();

    info!("Handling dbus requests");
    while CONTINUE.load(Ordering::SeqCst) {
        delay_for(Duration::from_millis(1000)).await;

        fan_daemon.step();

        let hpd = hpd();
        for i in 0..hpd.len() {
            if hpd[i] != last[i] && hpd[i] {
                info!("HotPlugDetect {}", i);
                c.send(Message::new_signal(DBUS_PATH, DBUS_NAME, "HotPlugDetect").unwrap().append1(i as u64))
                    .map_err(|()| "failed to send message".to_string())?;
            }
        }

        last = hpd;

        if let Ok(ref mux) = mux_res {
            unsafe {
                mux.step();
            }
        }
    }

    info!("daemon exited from loop");
    Ok(())
}

type MethodFut<'a, OA> = Pin<Box<dyn Future<Output = Result<OA, String>> + Send + 'a>>;

fn sync_method<IA, OA, F>(b: &mut IfaceBuilder<PowerDaemon>, name: &'static str, input_args: IA::strs, output_args: OA::strs, f: F)
    where 
          IA: arg::ArgAll + arg::ReadAll + Debug + Send + 'static,
          OA: arg::ArgAll + arg::AppendAll + 'static,
          F: Fn(&PowerDaemon, IA) -> MethodFut<'_, OA> + Send + Sync + Clone + 'static
{
    b.method_with_cr_async(name, input_args, output_args, move |mut ctx, cr, args| {
        info!("DBUS Received {}{:?} method", name, args);
        let daemon: Option<Arc<PowerDaemon>> = cr.data_mut(ctx.path()).cloned();
        let f = f.clone();
        async move {
            let reply = match daemon {
                Some(daemon) => {
                    let res: Result<OA, String> = { f(&daemon, args).await };
                    match res {
                        Ok(ret) => Ok(ret),
                        Err(err) => Err(MethodErr::failed(&err)),
                    }
                }
                None => Err(MethodErr::no_path(ctx.path()))
            };
            ctx.reply(reply)
        }
    });
}

/// DBus wrapper for a method taking no argument and returning no values
fn sync_action_method<F>(b: &mut IfaceBuilder<PowerDaemon>, name: &'static str, f: F)
    where F: Fn(&PowerDaemon) -> MethodFut<'_, ()> + Send + Sync + Clone + 'static
{
    sync_method(b, name, (), (), move |d, _: ()| f(d) );
}

/// DBus wrapper for method taking no arguments and returning one value
fn sync_get_method<T, F>(b: &mut IfaceBuilder<PowerDaemon>, name: &'static str, output_arg: &'static str, f: F)
    where T: arg::Arg + arg::Append + Debug + Send + 'static,
          F: Fn(& PowerDaemon) -> MethodFut<'_, T> + Send + Sync + Clone + 'static
{
    sync_method(b, name, (), (output_arg,), move |d, _: ()| {
        let res = f(d);
        Box::pin(Box::new(async move { res.await.map(|x| (x,)) }))
    });
}

/// DBus wrapper for method taking one argument and returning no values
fn sync_set_method<T, F>(b: &mut IfaceBuilder<PowerDaemon>, name: &'static str, input_arg: &'static str, f: F)
    where 
          T: arg::Arg + for<'z> arg::Get<'z> + Debug + Send + 'static,
          F: Fn(&PowerDaemon, T) -> MethodFut<'_, ()> + Send + Sync + Clone + 'static
{
    sync_method(b, name, (input_arg,), (), move |d, (arg,)| f(d, arg))
}
