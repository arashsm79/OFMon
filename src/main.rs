use std::collections::HashSet;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::{Duration, Instant};
use std::{fs, ops};

use embedded_hal_0_2_7::adc::OneShot;
use embedded_svc::http::server::registry::Registry;
use embedded_svc::http::server::{Request, Response};
use embedded_svc::http::SendStatus;
use embedded_svc::io::Read as SvcRead;
use embedded_svc::ipv4::{Ipv4Addr, Mask, RouterConfiguration, Subnet};
use embedded_svc::sys_time::SystemTime;
use embedded_svc::wifi::Wifi;
use embedded_svc::wifi::{AccessPointConfiguration, ApIpStatus, ApStatus, AuthMethod, Status};
use esp_idf_hal::adc::{Atten11dB, PoweredAdc, ADC1};
use esp_idf_hal::gpio::{Gpio34, Gpio35, Pins};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::netif::EspNetifStack;
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::nvs_storage::EspNvsStorage;

use esp_idf_svc::sysloop::EspSysLoopStack;
use esp_idf_svc::systime::EspSystemTime;
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys::{esp, gettimeofday, timeval};

use esp_idf_hal::adc;
use esp_idf_hal::prelude::Peripherals;

use anyhow::bail;
use cstr::cstr;
#[allow(unused_imports)]
use log::{debug, error, info, warn};

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

const ADC_BITS: u32 = 12;
const MAX_READING: u32 = 1 << ADC_BITS;
const MAX_MV_ATTEN_11: u16 = 2450;
const SUPPLY_VOLTAGE: f32 = 3.3;
const SAVE_PERIOD_TIMEOUT: u64 = 120; // 3600 for one hour
const MAX_SHARD_SIZE: u64 = 256; // in bytes
const CT_READING_SIZE: usize = 30; // in bytes
const MAX_SAMPLES: usize = 600;
const SAMPLE_ACCUMULATOR: [f32; MAX_SAMPLES] = [0.0; MAX_SAMPLES];
const NOISE_THRESHOLD: f32 = MAX_MV_ATTEN_11 as f32 / 8.0;
static ESP_SYSTEM_TIME: &EspSystemTime = &EspSystemTime {};

struct VoltagePin {
    pin: Gpio34<Atten11dB<ADC1>>,
    vcal: f32,
    phase_cal: f32,
    offset_v: f32,
}

struct CurrentPin {
    pin: Gpio35<Atten11dB<ADC1>>,
    ical: f32,
    offset_i: f32,
}

struct CT {
    id: u16,
    current_pin: CurrentPin,
    voltage_pin: VoltagePin,
    reading: CTReading,
}

#[derive(Debug)]
struct CTReading {
    real_power: f32,
    apparent_power: f32,
    i_rms: f32,
    v_rms: f32,
    kwh: f32,
    timestamp: u64,
}

struct CTStorage {
    readings_shard_counter: i32,
    readings_shards: HashSet<i32>,
}

static ACCESS_TOKEN: String = String::new();
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
    storage_lock.lock().unwrap().find_newest_readings_shard_num()?;

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
    let mut cts = init_adc(pins)?;
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
            save_period_start = Instant::now();
        }
        sleep(Duration::from_millis(1000));
    }
}

impl CTStorage {
    fn new() -> Self {
        CTStorage {
            readings_shard_counter: 1,
            readings_shards: HashSet::new(),
        }
    }

    /// Find the newest readings shard id
    ///
    /// under "/littlefs/ct_readings" files are saved with a number as their filename.
    /// here we iterate through all of them and find the newest file (the one with higher number as
    /// its filename). This is the file that we will be appending new data to.
    fn find_newest_readings_shard_num(&mut self) -> anyhow::Result<()> {
        let mut max_num = 1;
        if let Ok(paths) = fs::read_dir("/littlefs/ct_readings") {
            for path in paths {
                let num = path?.file_name().to_str().unwrap().parse()?;
                max_num = i32::max(max_num, num);
                self.readings_shards.insert(num);
            }
        } else {
            fs::create_dir("/littlefs/ct_readings")?;
        }
        Ok(())
    }

    /// Save sensor readings to storage.
    ///
    /// this function does not do any synchronization. If something like mutex is needed, you must deal
    /// with it before calling this function.
    /// under "/littlefs/ct_readings" files are saved with a number as their filename.
    /// newer files have a higher number as their filename.
    fn save_to_storage(
        &mut self,
        cts: &[CT; AC_PHASE],
    ) -> anyhow::Result<()> {
        // check whether the selected shard has enough size. if it doesn't create a new shard
        if ((MAX_SHARD_SIZE
            - fs::metadata(format!("/littlefs/ct_readings/{}", self.readings_shard_counter))?.len())
            as usize)
            < CT_READING_SIZE
        {
            self.readings_shard_counter += 1;
            self.readings_shards.insert(self.readings_shard_counter);
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(format!("/littlefs/ct_readings/{}", self.readings_shard_counter))?;
        info!(
            "Opened {} for writing.",
            format!("/littlefs/ct_readings/{}", self.readings_shard_counter)
        );

        // Append the readings for each CT at the end of the file
        for ct in cts {
            let buf = CTStorage::ct_reading_to_le_bytes(ct)?;
            file.write_all(&buf)?;
        }
        file.flush()?;
        Ok(())
    }

    fn ct_reading_to_le_bytes(ct: &CT) -> anyhow::Result<[u8; CT_READING_SIZE]> {
        let mut buf = [0_u8; CT_READING_SIZE];
        let mut pos = 0;
        pos += add_u16_to_buf(&ct.id, &mut buf, &pos)?;
        pos += add_f32_to_buf(&ct.reading.real_power, &mut buf, &pos)?;
        pos += add_f32_to_buf(&ct.reading.apparent_power, &mut buf, &pos)?;
        pos += add_f32_to_buf(&ct.reading.i_rms, &mut buf, &pos)?;
        pos += add_f32_to_buf(&ct.reading.v_rms, &mut buf, &pos)?;
        pos += add_f32_to_buf(&ct.reading.kwh, &mut buf, &pos)?;
        add_u64_to_buf(&ct.reading.timestamp, &mut buf, &pos)?;
        Ok(buf)
    }
}

fn add_u16_to_buf(val: &u16, buf: &mut [u8], offset: &usize) -> anyhow::Result<usize> {
    let bytes = val.to_le_bytes();
    let n = bytes.len();
    buf[*offset..(n + (*offset))].copy_from_slice(&bytes);
    Ok(n)
}

fn add_f32_to_buf(val: &f32, buf: &mut [u8], offset: &usize) -> anyhow::Result<usize> {
    let bytes = val.to_le_bytes();
    let n = bytes.len();
    buf[*offset..(n + (*offset))].copy_from_slice(&bytes);
    Ok(n)
}

fn add_u64_to_buf(val: &u64, buf: &mut [u8], offset: &usize) -> anyhow::Result<usize> {
    let bytes = val.to_le_bytes();
    let n = bytes.len();
    buf[*offset..(n + (*offset))].copy_from_slice(&bytes);
    Ok(n)
}

fn init_adc(pins: Pins) -> anyhow::Result<[CT; AC_PHASE]> {
    #[cfg(feature = "single-phase")]
    {
        Ok([CT {
            id: 1,
            current_pin: CurrentPin {
                pin: pins.gpio35.into_analog_atten_11db()?,
                ical: 102.0,
                offset_i: 1066.0,
            },
            voltage_pin: VoltagePin {
                pin: pins.gpio34.into_analog_atten_11db()?,
                vcal: 232.5,
                phase_cal: 1.7,
                offset_v: 1288.0,
            },
            reading: CTReading {
                i_rms: 0.0,
                v_rms: 0.0,
                timestamp: 0,
                real_power: 0.0,
                apparent_power: 0.0,
                kwh: 0.0,
            },
        }])
    }
    #[cfg(feature = "three-phase")]
    {
        Ok([
            CT {
                id: 1,
                current_pin: CurrentPin {
                    pin: pins.gpio32.into_analog_atten_11db()?,
                    ical: 30.0,
                    offset_i: 1066.0,
                },
                voltage_pin: VoltagePin {
                    pin: pins.gpio39.into_analog_atten_11db()?,
                    vcal: 219.25,
                    phase_cal: 1.7,
                    offset_v: 1288.0,
                },
                reading: CTReading {
                    i_rms: 0.0,
                    v_rms: 0.0,
                    timestamp: 0,
                    real_power: 0.0,
                    apparent_power: 0.0,
                    kwh: 0.0,
                },
            },
            CT {
                id: 2,
                current_pin: CurrentPin {
                    pin: pins.gpio35.into_analog_atten_11db()?,
                    ical: 30.0,
                    offset_i: 1066.0,
                },
                voltage_pin: VoltagePin {
                    pin: pins.gpio36.into_analog_atten_11db()?,
                    vcal: 219.25,
                    phase_cal: 1.7,
                    offset_v: 1288.0,
                },
                reading: CTReading {
                    i_rms: 0.0,
                    v_rms: 0.0,
                    timestamp: 0,
                    real_power: 0.0,
                    apparent_power: 0.0,
                    kwh: 0.0,
                },
            },
            CT {
                id: 3,
                current_pin: CurrentPin {
                    pin: pins.gpio34.into_analog_atten_11db()?,
                    ical: 30.0,
                    offset_i: 1066.0,
                },
                voltage_pin: VoltagePin {
                    pin: pins.gpio33.into_analog_atten_11db()?,
                    vcal: 219.25,
                    phase_cal: 1.7,
                    offset_v: 1288.0,
                },
                reading: CTReading {
                    i_rms: 0.0,
                    v_rms: 0.0,
                    timestamp: 0,
                    real_power: 0.0,
                    apparent_power: 0.0,
                    kwh: 0.0,
                },
            },
        ])
    }
}

impl CT {
    fn calculate_energy(
        &mut self,
        powered_adc1: &mut PoweredAdc<ADC1>,
        crossing: u32,
        timeout: std::time::Duration,
    ) -> anyhow::Result<()> {
        // Variables
        let mut cross_count = 0;
        let mut n_samples: u32 = 0;

        // Used for delay/phase compensation
        let mut filtered_v = 0.0;
        let mut last_filtered_v = 0.0;
        let mut filtered_i = 0.0;
        let mut last_filtered_i = 0.0;

        let mut sample_v: u16 = 0;
        let mut sample_i: u16 = 0;
        let mut offset_v: f32 = self.voltage_pin.offset_v as f32;
        let mut offset_i: f32 = self.current_pin.offset_i as f32;

        let mut min_sample_i: u16 = MAX_MV_ATTEN_11;
        let mut min_sample_v: u16 = MAX_MV_ATTEN_11;
        let mut max_sample_i: u16 = 0;
        let mut max_sample_v: u16 = 0;

        let (mut sum_v, mut sum_i, mut sum_p) = (0.0, 0.0, 0.0);
        let mut check_v_cross = false;
        let mut last_v_cross;

        let mut start = std::time::Instant::now(); // start.elapsed() makes sure it doesnt get stuck in the loop if there is an error.
        let mut start_v = 0;

        // 1) Waits for the waveform to be close to 'zero' (mid-scale adc) part in sin curve.
        loop {
            start_v = powered_adc1
                .read(&mut self.voltage_pin.pin)
                .unwrap_or(start_v);

            if ((start_v as f32) < MAX_MV_ATTEN_11 as f32 * 0.55)
                && ((start_v as f32) > MAX_MV_ATTEN_11 as f32 * 0.45)
            {
                break;
            }
            if start.elapsed() > timeout {
                break;
            }
        }
        // 2) Main measurement loop
        start = std::time::Instant::now();
        while (cross_count < crossing) && (start.elapsed() < timeout) {
            // A) Read in raw voltage and current samples
            sample_i = powered_adc1
                .read(&mut self.current_pin.pin)
                .unwrap_or(sample_i);
            sample_v = powered_adc1
                .read(&mut self.voltage_pin.pin)
                .unwrap_or(sample_v);

            // B) Apply digital low pass filters to extract the 2.5 V or 1.65 V dc offset,
            //     then subtract this - signal is now centred on 0 counts.
            offset_i = offset_i + ((sample_i as f32 - offset_i) / 512.0);
            filtered_i = sample_i as f32 - offset_i;

            offset_v = offset_v + ((sample_v as f32 - offset_v) / 512.0);
            filtered_v = sample_v as f32 - offset_v;

            // Ignore noise
            if f32::abs(last_filtered_v - filtered_v) < NOISE_THRESHOLD {
                min_sample_v = u16::min(min_sample_v, sample_v);
                max_sample_v = u16::max(max_sample_v, sample_v);
            }
            if f32::abs(last_filtered_i - filtered_i) < NOISE_THRESHOLD {
                min_sample_i = u16::min(min_sample_i, sample_i);
                max_sample_i = u16::max(max_sample_i, sample_i);
            }

            // C) RMS
            sum_v += filtered_v * filtered_v;
            sum_i += filtered_i * filtered_i;

            // E) Phase calibration
            let phase_shift_v =
                last_filtered_v + self.voltage_pin.phase_cal * (filtered_v - last_filtered_v);

            // F) Instantaneous power calc
            sum_p += phase_shift_v * filtered_i;

            // G) Find the number of times the voltage has crossed the initial voltage
            //    - every 2 crosses we will have sampled 1 wavelength
            //    - so this method allows us to sample an integer number of half wavelengths which increases accuracy
            last_v_cross = check_v_cross;
            if sample_v > start_v {
                check_v_cross = true;
            } else {
                check_v_cross = false;
            }
            if n_samples == 0 {
                last_v_cross = check_v_cross;
            }

            if last_v_cross != check_v_cross {
                cross_count += 1;
            }

            n_samples += 1;
            last_filtered_v = filtered_v;
            last_filtered_i = filtered_i;
        }

        // Improve the approximation for mid point (dc offset)
        offset_i = (offset_i + ((max_sample_i + min_sample_i) as f32 / 2.0)) / 2.0;
        offset_v = (offset_v + ((max_sample_v + min_sample_v) as f32 / 2.0)) / 2.0;

        self.current_pin.offset_i = offset_i;
        self.voltage_pin.offset_v = offset_v;

        let v_ratio = self.voltage_pin.vcal * (SUPPLY_VOLTAGE / (MAX_MV_ATTEN_11 as f32));
        let v_rms = v_ratio * f32::sqrt(sum_v / n_samples as f32);

        let i_ratio = self.current_pin.ical * (SUPPLY_VOLTAGE / (MAX_MV_ATTEN_11 as f32));
        let i_rms = i_ratio * f32::sqrt(sum_i / n_samples as f32);

        // Calculate power values
        let real_power = f32::abs(v_ratio * i_ratio * (sum_p / n_samples as f32));
        let apparent_power = v_rms * i_rms;
        let kwh = real_power * start.elapsed().as_secs_f32() / SAVE_PERIOD_TIMEOUT as f32;
        let new_reading = CTReading {
            real_power,
            apparent_power,
            kwh,
            i_rms,
            v_rms,
            timestamp: now().as_millis() as u64,
        };
        self.reading += new_reading;
        info!("Current offset: {}", offset_i);
        info!("Vol offset: {}", offset_v);
        info!("n_samples: {}", n_samples);
        info!("crossing: {}", cross_count);
        info!("dur: {}", start.elapsed().as_millis());
        Ok(())
    }
}

impl ops::AddAssign<CTReading> for CTReading {
    fn add_assign(&mut self, rhs: CTReading) {
        self.i_rms = (self.i_rms + rhs.i_rms) / 2.0;
        self.v_rms = (self.v_rms + rhs.v_rms) / 2.0;
        self.real_power = (self.real_power + rhs.real_power) / 2.0;
        self.apparent_power = (self.apparent_power + rhs.apparent_power) / 2.0;
        self.kwh = self.kwh + rhs.kwh;
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
fn init_web_server() -> anyhow::Result<EspHttpServer> {
    let mut server = EspHttpServer::new(&Default::default())?;

    server.handle_get("/", |req, mut res| {
        println!("{:#?}", req.query_string());
        res.set_ok();
        res.send_str(&templated_webpage("You should not be here."))?;
        Ok(())
    })?;

    server.handle_get("/telemetry", |req, mut res| {
        println!("{:#?}", req.query_string());
        res.set_ok();
        res.send_str(&templated_webpage("You should not be here."))?;
        Ok(())
    })?;

    server.handle_get("/time", |mut req, mut res| {
        println!("{:#?}", req.query_string());
        let mut buf = [0_u8; 256];
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
        println!("Response: {}", std::str::from_utf8(&buf)?);
        // get time, set it, return parsed time as response.
        res.set_ok();
        res.send_str(&templated_webpage("You should not be here."))?;
        Ok(())
    })?;

    server.handle_get("/token", |req, mut res| {
        println!("{:#?}", req.query_string());
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
