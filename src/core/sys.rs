use std::sync::OnceLock;

use anyhow::Result;
use regex::Regex;
use sysinfo::System;

use crate::core::fs::{Sizes, size_as};

const SYSTEM: OnceLock<System> = OnceLock::new();

pub fn kernel_version() -> Result<String> {
    let regex: Regex = Regex::new("-.*")?;

    let k_ver = System::kernel_version().unwrap();
    Ok((&regex.replace_all(&k_ver, "")).to_string())
}

pub fn get_available_storage(device: &str, unit: Sizes) -> Result<f64> {
    todo!()
}

pub fn required_storage_available(device: &str, estimated_size: f64, unit: Sizes) -> Result<bool> {
    let bytes = size_as(estimated_size, unit, Sizes::Bytes);
    todo!()
}

fn primary_str_dev_name() -> String {
    todo!()
}
