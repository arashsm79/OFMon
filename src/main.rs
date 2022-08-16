mod ct;
pub(crate) mod utils;

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};

use embedded_svc::http::server::registry::Registry;
use embedded_svc::http::server::{Request, Response};
use embedded_svc::http::SendStatus;
use embedded_svc::io::{Read as SvcRead, Write as SvcWrite};
use embedded_svc::ipv4::{Ipv4Addr, Mask, RouterConfiguration, Subnet};
use embedded_svc::wifi::Wifi;
use embedded_svc::wifi::{AccessPointConfiguration, ApIpStatus, ApStatus, AuthMethod, Status};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::netif::EspNetifStack;
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::nvs_storage::EspNvsStorage;

use esp_idf_svc::sysloop::EspSysLoopStack;
use esp_idf_svc::systime::EspSystemTime;
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys::{esp, gettimeofday, settimeofday, timeval};

use esp_idf_hal::adc;
use esp_idf_hal::prelude::Peripherals;

use anyhow::bail;
use cstr::cstr;
#[allow(unused_imports)]
use log::{debug, error, info, warn};

use crate::ct::{CTStorage, CT};

// const SINGLE_PHASE_CURRENT_PIN: u8 = 35;
// const SINGLE_PHASE_VOLTAGE_PIN: u8 = 34;
// const THREE_PHASE_CURRENT_PINS: [u8; 3] = [32, 35, 34];
// const THREE_PHASE_VOLTAGE_PINS: [u8; 3] = [39, 36, 33];
// const LED_PIN: u8 = 14;
// const DC_VOLTAGE: [u16; 3] = [1892; 3];
// const DC_CURRENT: [u16; 3] = [1635; 3];
// const CURRENT_SCALE: [f32; 3] = [102.0; 3]; //111.1;
// const VOLTAGE_SCALE: [f32; 3] = [232.5; 3];

/// Specify the number of CT modules that will be connected
/// to this system.
#[cfg(feature = "single-phase")]
const AC_PHASE: usize = 1;
#[cfg(feature = "three-phase")]
const AC_PHASE: usize = 3;

// ADC constants
const ADC_BITS: u32 = 12;
const MAX_READING: u32 = 1 << ADC_BITS;
const MAX_MV_ATTEN_11: u16 = 2450;
const SUPPLY_VOLTAGE: f32 = 3.3;
const NOISE_THRESHOLD: f32 = MAX_MV_ATTEN_11 as f32 / 8.0;

// Periodic actions constants
const SAVE_PERIOD_TIMEOUT: u64 = 120; // 3600 for one hour

// Storage constants
const MAX_SHARD_SIZE: u64 = 256; // in bytes
const MAX_TIME_STORAGE_SIZE: u64 = 256; // in bytes
const CT_READING_SIZE: usize = 30; // in bytes

// Network constants
const ACCESS_TOKEN_SIZE: usize = 20;
const GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    // return Ok(());
    // Initialize LittleFS storage
    // let _fs_conf = init_littlefs_storage()?;
    info!("Initialized and mounted littlefs storage.");

    // Initialize CT readings shards
    let storage_lock = Arc::new(Mutex::new(CTStorage::new()));
    storage_lock
        .lock()
        .unwrap()
        .find_newest_readings_shard_num()?;

    // Initialize NVS storage
    // let (default_nvs, _keystore) = init_nvs_storage()?;
    info!("Initialized default NVS storage.");

    // SSID and password for the Wifi access point.
    let mut ap_ssid: String = String::new();
    let ap_password: &str = "12345678";
    // configure_access_point_ssid(&mut ap_ssid)?;
    info!("Configured AP SSID as: {}.", ap_ssid);

    // let _wifi = init_access_point(&ap_ssid, ap_password, default_nvs)?;
    info!("Initialized Wifi.");

    // let _web_server = init_web_server()?;
    info!("Initialized Web Server.");

    // Initilize peripherals and pins
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // Initilize ADC
    let mut powered_adc1 = adc::PoweredAdc::new(
        peripherals.adc1,
        adc::config::Config::new().calibration(false),
    )?;
    let mut cts = CT::init(pins)?;
    info!("Initialized ADC 1.");

    // Main Loop
    let mut save_period_start = Instant::now();
    loop {
        for ct in &mut cts {
            ct.calculate_energy(&mut powered_adc1, 200, std::time::Duration::new(3, 0))?;
            info!("Energy Reading: {:?}", ct.reading);
        }

        // save the readings of CTs to storage.
        if save_period_start.elapsed() > Duration::new(SAVE_PERIOD_TIMEOUT, 0) {
            info!("Saving to storage.");
            let mut ct_storage = match storage_lock.lock() {
                Ok(gaurd) => gaurd,
                Err(poisoned) => poisoned.into_inner(),
            };
            ct_storage.save_to_storage(&cts)?;
            ct_storage.store_time(now().as_millis() as u64)?;

            // Reset CT readings.
            for ct in &mut cts {
                ct.reset();
            }
            save_period_start = Instant::now();
        }
        sleep(Duration::from_millis(1000));
    }
}

/// Initializes a littlefs file system.
///
/// A partition with name `LITTLEFS_PARTITION_NAME` has to be specified
/// in the partition table csv file.
fn init_littlefs_storage() -> anyhow::Result<esp_idf_sys::esp_vfs_littlefs_conf_t> {
    let mut fs_conf = esp_idf_sys::esp_vfs_littlefs_conf_t {
        base_path: cstr!("/littlefs").as_ptr(),
        partition_label: cstr!("littlefs").as_ptr(),
        ..Default::default()
    };
    fs_conf.set_format_if_mount_failed(true as u8);
    fs_conf.set_dont_mount(false as u8);

    unsafe { esp!(esp_idf_sys::esp_vfs_littlefs_register(&fs_conf))? };
    let (mut fs_total_bytes, mut fs_used_bytes) = (0, 0);
    unsafe {
        esp!(esp_idf_sys::esp_littlefs_info(
            fs_conf.partition_label,
            &mut fs_total_bytes,
            &mut fs_used_bytes
        ))?
    };
    info!(
        "LittleFs Info: total bytes = {}, used bytes = {}.",
        fs_total_bytes, fs_used_bytes
    );

    Ok(fs_conf)
}

/// Initializes a nvs file system.
///
/// A partition with name `NVS_PARTITION_NAME` has to be specified
/// in the partition table csv file.
fn init_nvs_storage() -> anyhow::Result<(Arc<EspDefaultNvs>, Arc<EspNvsStorage>)> {
    let default_nvs = Arc::new(EspDefaultNvs::new()?);
    let keystore = Arc::new(EspNvsStorage::new_default(default_nvs.clone(), "f", true)?);
    Ok((default_nvs, keystore))
}

/// Sets the value of `ap_ssid` as a combination of this
/// device MAC address and a custom string.
fn configure_access_point_ssid(ap_ssid: &mut String) -> anyhow::Result<()> {
    let mut mac = [0u8; 6];
    esp!(unsafe {
        esp_idf_sys::esp_read_mac(
            mac.as_mut_ptr() as *mut _,
            esp_idf_sys::esp_mac_type_t_ESP_MAC_WIFI_SOFTAP,
        )
    })?;
    ap_ssid.push_str("SEM-");
    ap_ssid.push_str(
        format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
        )
        .as_str(),
    );
    Ok(())
}

/// Initializes access point with the given ssid and pasword.
///
/// Authentication method is WPA2Personal.
fn init_access_point(
    ssid: &str,
    password: &str,
    default_nvs: Arc<EspDefaultNvs>,
) -> anyhow::Result<Box<EspWifi>> {
    let netif_stack = Arc::new(EspNetifStack::new()?);
    let sys_loop_stack = Arc::new(EspSysLoopStack::new()?);

    let mut wifi = Box::new(EspWifi::new(netif_stack, sys_loop_stack, default_nvs)?);
    wifi.set_configuration(&embedded_svc::wifi::Configuration::AccessPoint(
        AccessPointConfiguration {
            ssid: ssid.into(),
            password: password.into(),
            auth_method: AuthMethod::WPA2Personal,
            ip_conf: Some(RouterConfiguration {
                subnet: Subnet {
                    gateway: GATEWAY_IP,
                    mask: Mask(24),
                },
                dhcp_enabled: true,
                dns: None,
                secondary_dns: None,
            }),
            ..Default::default()
        },
    ))?;

    let status = wifi.get_status();

    wifi.wait_status_with_timeout(Duration::from_secs(20), |status| !status.is_transitional())
        .map_err(|e| anyhow::anyhow!("Unexpected Wifi status: {:?}", e))?;

    if let Status(_, ApStatus::Started(ApIpStatus::Done)) = status {
        info!("Wifi Status: {:?}", status);
    } else {
        bail!("Unexpected Wifi status: {:?}", status);
    }

    Ok(wifi)
}

/// Initilizes the web server and registers some handlers.
fn init_web_server(storage_lock: Arc<Mutex<CTStorage>>) -> anyhow::Result<EspHttpServer> {
    let mut server = EspHttpServer::new(&Default::default())?;

    let handler_storage_lock = storage_lock.clone();
    server.handle_get("/", | _req, mut res| {
        res.set_ok();
        res.send_str(&templated_webpage("You should not be here."))?;
        Ok(())
    })?;

    let handler_storage_lock = storage_lock.clone();
    server.handle_get("/telemetry", move | _req, mut res| {
        res.set_ok();
        let mut writer = res.into_writer()?;
        {
            let mut ct_storage = match handler_storage_lock.lock() {
                Ok(gaurd) => gaurd,
                Err(poisoned) => poisoned.into_inner(),
            };
            ct_storage.send_readings_shards(&mut writer)?;
        }
        Ok(())
    })?;

    let handler_storage_lock = storage_lock.clone();
    server.handle_get("/time", move |mut req, mut res| {
        let mut buf = [0_u8; std::mem::size_of::<u64>()];
        let mut size = 0;
        let mut reader = req.reader();
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            size += n;
        }
        println!("Read {} bytes of data", size);

        // Convert raw bytes to time. Store it and update system time.
        let time = u64::from_le_bytes(buf);
        {
            let mut ct_storage = match handler_storage_lock.lock() {
                Ok(gaurd) => gaurd,
                Err(poisoned) => poisoned.into_inner(),
            };
            ct_storage.store_time(time)?;
            println!("Response: {}", time);
        }
        set_system_time(time)?;

        res.set_ok();
        Ok(())
    })?;

    let handler_storage_lock = storage_lock.clone();
    server.handle_get("/token", move |mut req, mut res| {
        let mut buf = [0_u8; ACCESS_TOKEN_SIZE];
        let mut size = 0;
        let mut reader = req.reader();
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            size += n;
        }
        println!("Read {} bytes of data", size);

        let token = buf;
        // Store the given token in storage.
        {
            let mut ct_storage = match handler_storage_lock.lock() {
                Ok(gaurd) => gaurd,
                Err(poisoned) => poisoned.into_inner(),
            };
            ct_storage.store_token(&token)?;
            println!("Response: {}", std::str::from_utf8(&token)?);
        }
        res.set_ok();
        Ok(())
    })?;

    Ok(server)
}

fn templated_webpage(content: impl AsRef<str>) -> String {
    format!(
        r#"
<!DOCTYPE html>
<html>
    <head>
        <meta charset="utf-8">
        <title>esp-rs web server</title>
    </head>
    <body>
        {}
    </body>
</html>
"#,
        content.as_ref()
    )
}

/// Calculates the RMS value for a given slice of samples.
#[allow(dead_code)]
fn calc_rms(samples: &[f32], size: usize) -> f32 {
    (samples[..size].iter().fold(0.0, |sum, &x| sum + (x * x)) / size as f32).sqrt()
}

fn now() -> Duration {
    let mut tv_now: timeval = Default::default();

    unsafe {
        gettimeofday(&mut tv_now as *mut _, core::ptr::null_mut());
    }

    Duration::from_micros(tv_now.tv_sec as u64 * 1000000_u64 + tv_now.tv_usec as u64)
}

fn set_system_time(time: u64) -> anyhow::Result<()> {
    let mut tv_now: timeval = timeval {
        tv_sec: time as i32,
        tv_usec: 0,
    };
    esp!(unsafe { settimeofday(&mut tv_now as *mut _, core::ptr::null_mut()) });
    Ok(())
}
