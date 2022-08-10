use std::fs;
use std::io::{Read, Write};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use embedded_hal_0_2_7::adc::OneShot;
use embedded_svc::http::server::registry::Registry;
use embedded_svc::http::server::{Request, Response};
use embedded_svc::http::SendStatus;
use embedded_svc::io::Read as SvcRead;
use embedded_svc::ipv4::{Ipv4Addr, Mask, RouterConfiguration, Subnet};
use embedded_svc::wifi::Wifi;
use embedded_svc::wifi::{AccessPointConfiguration, ApIpStatus, ApStatus, AuthMethod, Status};
use esp_idf_hal::adc::{Atten11dB, PoweredAdc, ADC1};
use esp_idf_hal::gpio::{Gpio34, Gpio35, Pins};
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::netif::EspNetifStack;
use esp_idf_svc::nvs::EspDefaultNvs;
use esp_idf_svc::nvs_storage::EspNvsStorage;

use esp_idf_svc::sysloop::EspSysLoopStack;
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys::esp;

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
const DC_VOLTAGE: [u16; 3] = [1892; 3];
const DC_CURRENT: [u16; 3] = [1635; 3];
const CURRENT_SCALE: [f32; 3] = [102.0; 3]; //111.1;
const VOLTAGE_SCALE: [f32; 3] = [232.5; 3];
const MAX_SAMPLES: usize = 300;
const AC_PHASE: u8 = 1;
const ADC_COUNT: u32 = 4095;

const SAMPLE_ACCUMULATOR: [f32; MAX_SAMPLES] = [0.0; MAX_SAMPLES];

struct VoltagePin {
    pin: Gpio34<Atten11dB<ADC1>>,
    vcal: f32,
    phase_cal: f32,
    offset_v: u32,
}

struct CurrentPin {
    pin: Gpio35<Atten11dB<ADC1>>,
    ical: f32,
    offset_i: u32,
}

struct CT {
    current_pin: CurrentPin,
    voltage_pin: VoltagePin,
    current_samples: [f32; MAX_SAMPLES],
    voltage_samples: [f32; MAX_SAMPLES],
    sample_count: usize,
}

#[allow(dead_code)]
static ACCESS_TOKEN: String = String::new();
const GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    // return Ok(());
    // Initialize LittleFS storage
    let _fs_conf = init_littlefs_storage()?;
    info!("Initialized and mounted littlefs storage.");

    // Initialize NVS storage
    let (default_nvs, _keystore) = init_nvs_storage()?;
    info!("Initialized default NVS storage.");

    // SSID and password for the Wifi access point.
    let mut ap_ssid: String = String::new();
    let ap_password: &str = "12345678";
    configure_access_point_ssid(&mut ap_ssid)?;
    info!("Configured AP SSID as: {}.", ap_ssid);

    let _wifi = init_access_point(&ap_ssid, ap_password, default_nvs)?;
    info!("Initialized Wifi.");

    let _web_server = init_web_server()?;
    info!("Initialized Web Server.");

    // Initilize peripherals and pins
    let peripherals = Peripherals::take().unwrap();
    let mut pins = peripherals.pins;

    // Initilize ADC

    let mut powered_adc1 = adc::PoweredAdc::new(
        peripherals.adc1,
        adc::config::Config::new().calibration(false),
    )?;
    let mut cts = init_adc(pins)?;

    let ac_phase = 1;
    let mut current_samples = [0.0; MAX_SAMPLES];
    let mut voltage_samples = [0.0; MAX_SAMPLES];
    let mut sample_count = 0;

    loop {
        // Read current and voltage values, calibrate them
        // and add them to the samples array
        for ct in &mut cts {
            ct.calculate_energy(&mut powered_adc1, 20, time::Duration::new(5, 0))?;
        }
//         let raw_current_reading =
//          powered_adc1.read(&mut cts[0].current_pin.pin).unwrap() - DC_CURRENT[ac_phase];
        //         let current_reading =
        //             CURRENT_SCALE[ac_phase] * ((raw_current_reading as f32 * 3.3) / 4095.0);
        //
        //         let raw_voltage_reading =
        //             powered_adc1.read(&mut single_phase_voltage_pin).unwrap() - DC_VOLTAGE[ac_phase];
        //         let voltage_reading =
        //             VOLTAGE_SCALE[ac_phase] * ((raw_voltage_reading as f32 * 3.3) / 4095.0);
        //
        //         current_samples[sample_count] = current_reading;
        //         voltage_samples[sample_count] = voltage_reading;
        //
        //         sample_count += 1;
        //
        //         info!("Current: {}", current_reading);
        //         info!("Voltage: {}", voltage_reading);
        //
        //         if sample_count >= MAX_SAMPLES {
        //             let current_rms = calc_rms(&current_samples, sample_count);
        //             let voltage_rms = calc_rms(&voltage_samples, sample_count);
        //             sample_count = 0;
        //             info!("Current RMS : {}", current_rms);
        //             info!("Voltage RMS : {}", voltage_rms);
        //         }
        //
        sleep(Duration::from_millis(1000));
    }
}

fn init_adc(pins: Pins) -> anyhow::Result<[CT; AC_PHASE as usize]> {
    #[cfg(feature = "single-phase")]
    {
        Ok([CT {
            current_pin: CurrentPin {
                pin: pins.gpio35.into_analog_atten_11db()?,
                ical: 3.3,
                offset_i: 1000,
            },
            voltage_pin: VoltagePin {
                pin: pins.gpio34.into_analog_atten_11db()?,
                vcal: 3.3,
                phase_cal: 1.2,
                offset_v: 1000,
            },
            sample_count: 0,
            current_samples: [0.0; MAX_SAMPLES],
            voltage_samples: [0.0; MAX_SAMPLES],
        }])
    }
    #[cfg(feature = "three-phase")]
    {
        Ok([
            CT {
                current_pin: CurrentPin {
                    pin: pins.gpio32.into_analog_atten_11db()?,
                    ical: 3.3,
                    offset_i: 1000,
                },
                voltage_pin: VoltagePin {
                    pin: pins.gpio39.into_analog_atten_11db()?,
                    vcal: 3.3,
                    phase_cal: 1.2,
                    offset_v: 1000,
                },
                sample_count: 0,
                current_samples: [0.0; MAX_SAMPLES],
                voltage_samples: [0.0; MAX_SAMPLES],
            },
            CT {
                current_pin: CurrentPin {
                    pin: pins.gpio35.into_analog_atten_11db()?,
                    ical: 3.3,
                    offset_i: 1000,
                },
                voltage_pin: VoltagePin {
                    pin: pins.gpio36.into_analog_atten_11db()?,
                    vcal: 3.3,
                    phase_cal: 1.2,
                    offset_v: 1000,
                },
                sample_count: 0,
                current_samples: [0.0; MAX_SAMPLES],
                voltage_samples: [0.0; MAX_SAMPLES],
            },
            CT {
                current_pin: CurrentPin {
                    pin: pins.gpio34.into_analog_atten_11db()?,
                    ical: 3.3,
                    offset_i: 1000,
                },
                voltage_pin: VoltagePin {
                    pin: pins.gpio33.into_analog_atten_11db()?,
                    vcal: 3.3,
                    phase_cal: 1.2,
                    offset_v: 1000,
                },
                sample_count: 0,
                current_samples: [0.0; MAX_SAMPLES],
                voltage_samples: [0.0; MAX_SAMPLES],
            },
        ])
    }
}

impl CT {
    fn calculate_energy(
        &mut self,
        powered_adc1: &mut PoweredAdc<ADC1>,
        crossing: u32,
        timeout: time::Duration,
    ) -> anyhow::Result<()> {
        let supply_voltage = 3.3;
        let mut start = time::Instant::now(); // start.elapsed() makes sure it doesnt get stuck in the loop if there is an error.
        let start_v = 0.0;
        // 1) Waits for the waveform to be close to 'zero' (mid-scale adc) part in sin curve.
        loop {
            if (start_v < ADC_COUNT as f32 * 0.55) && (start_v > ADC_COUNT as f32 * 0.45) {
                break;
            }
            if start.elapsed() > timeout {
                break;
            }
        }

        // 2) Main measurement loop
        let mut cross_count = 0;
        let mut n_samples = 0;

        // Used for delay/phase compensation
        let mut filtered_v = 0.0;
        let mut last_filtered_v = 0.0;
        let mut filtered_i = 0.0;

        let mut phase_shift_v = 0.0;

        let mut sample_v: u16 = 0;
        let mut sample_i: u16 = 0;
        let mut offset_v: f32 = 0.0;
        let mut offset_i: f32 = 0.0;

        let (mut sq_v, mut sum_v, mut sq_i, mut sum_i, mut inst_p, mut sum_p) =
            (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        let (mut last_v_cross, mut check_v_cross) = (true, true);

        start = time::Instant::now();
        while (cross_count < crossing) && (start.elapsed() > timeout) {
            // A) Read in raw voltage and current samples
            sample_i = powered_adc1
                .read(&mut self.current_pin.pin)
                .unwrap_or(sample_i);
            sample_v = powered_adc1
                .read(&mut self.voltage_pin.pin)
                .unwrap_or(sample_v);

            // B) Apply digital low pass filters to extract the 2.5 V or 1.65 V dc offset,
            //     then subtract this - signal is now centred on 0 counts.
            offset_i = offset_i + ((sample_i as f32 - offset_i) / 1024.0);
            filtered_i = sample_i as f32 - offset_i;

            offset_v = offset_v + ((sample_v as f32 - offset_v) / 1024.0);
            filtered_v = sample_v as f32 - offset_v;

            // C) RMS
            sq_v = filtered_v * filtered_v;
            sum_v += sq_v;

            sq_i = filtered_i * filtered_i;
            sum_i += sq_i;

            // E) Phase calibration
            phase_shift_v =
                last_filtered_v + self.voltage_pin.phase_cal * (filtered_v - last_filtered_v);

            // F) Instantaneous power calc
            sum_p += phase_shift_v * filtered_i;

            // G) Find the number of times the voltage has crossed the initial voltage
            //    - every 2 crosses we will have sampled 1 wavelength
            //    - so this method allows us to sample an integer number of half wavelengths which increases accuracy
            last_v_cross = check_v_cross;
            if sample_v as f32 > start_v {
                check_v_cross = true;
            } else {
                check_v_cross = false;
            }
            if n_samples == 1 {
                last_v_cross = check_v_cross;
            }

            if last_v_cross != check_v_cross {
                cross_count += 1;
            }
        }

        let v_ratio = self.voltage_pin.vcal * (3.3 / (ADC_COUNT as f32));
        let v_rms = v_ratio * f32::sqrt(sum_v / n_samples as f32);

        let i_ratio = self.current_pin.ical * (3.3 / (ADC_COUNT as f32));
        let i_rms = i_ratio * f32::sqrt(sum_i / n_samples as f32);

        // Calculate power values
        let real_power = v_ratio * i_ratio * sum_p / n_samples as f32;
        let apparent_power = v_rms * i_rms;
        let power_factor = real_power / apparent_power;

        Ok(())
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
fn calc_rms(samples: &[f32], size: usize) -> f32 {
    (samples[..size].iter().fold(0.0, |sum, &x| sum + (x * x)) / size as f32).sqrt()
}
