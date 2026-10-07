use crate::core::fs::{Sizes, size_as};
use anyhow::{Context, Result};
use std::sync::OnceLock;
use sysinfo::{Disks, System};
static SYSTEM: OnceLock<System> = OnceLock::new();
pub fn kernel_version() -> Result<String> {
    System::kernel_version()
        .context("Kernel version unavailable")
        .map(|v| v.split('-').next().unwrap_or(&v).to_string())
}
pub fn get_available_storage(device: &str, unit: Sizes) -> Result<f64> {
    let disks = Disks::new_with_refreshed_list();
    let disk = disks
        .list()
        .iter()
        .find(|d| d.mount_point().to_string_lossy() == device)
        .or_else(|| {
            disks
                .list()
                .iter()
                .find(|d| d.mount_point().to_string_lossy() == "/")
        });
    let disk = disk.context("Storage device not found")?;
    Ok(size_as(disk.available_space() as f64, Sizes::Bytes, unit))
}
pub fn required_storage_available(device: &str, estimated_size: f64, unit: Sizes) -> Result<bool> {
    Ok(get_available_storage(device, Sizes::Bytes)? >= size_as(estimated_size, unit, Sizes::Bytes))
}
fn primary_str_dev_name() -> String {
    Disks::new_with_refreshed_list()
        .list()
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"))
        .map(|d| d.mount_point().display().to_string())
        .unwrap_or_else(|| "/".into())
}
