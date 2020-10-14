use std::{
    fs,
    path::Path,
};

use crate::err_str;

const START_THRESHOLD: &str = "/sys/class/power_supply/BAT0/charge_control_start_threshold";
const END_THRESHOLD: &str = "/sys/class/power_supply/BAT0/charge_control_end_threshold";

fn is_s76_ec() -> bool {
    Path::new("/sys/bus/acpi/devices/17761776:00").is_dir()
}

pub(crate) fn get_charge_thresholds() -> Result<(i32, i32), String> {
    // XXX check if system76 hardware (for now?)
    match (fs::read_to_string(START_THRESHOLD), fs::read_to_string(END_THRESHOLD)) {
        (Ok(start), Ok(end)) => {
            let start = i32::from_str_radix(&start, 10).map_err(err_str)?;
            let end = i32::from_str_radix(&end, 10).map_err(err_str)?;
            Ok((start, end))
        }
        _ => Err("".to_string())
    }
}

pub(crate) fn set_charge_thresholds((start, end): (i32, i32)) -> Result<(), String> {
    // XXX check if system76 hardware (for now?)
    fs::write(START_THRESHOLD, format!("{}", start)).map_err(err_str)?;
    fs::write(END_THRESHOLD, format!("{}", end)).map_err(err_str)?;
    Ok(())
}