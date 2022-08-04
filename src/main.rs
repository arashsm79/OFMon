use std::str::FromStr;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use anyhow::bail;
use embedded_hal_0_2_7::adc::OneShot;
use embedded_svc::http::server::registry::Registry;
use embedded_svc::http::server::{Request, Response};
use embedded_svc::http::SendStatus;
use embedded_svc::ipv4::{Ipv4Addr, Mask, RouterConfiguration, Subnet};
use embedded_svc::storage::RawStorage;
use embedded_svc::wifi::{
    AccessPointConfiguration, ApIpStatus, ApStatus, AuthMethod, ClientConfiguration, Status,
};
use embedded_svc::wifi::{ClientConnectionStatus, ClientIpStatus, ClientStatus, Wifi};
use esp_idf_hal::prelude::Peripherals;
use esp_idf_svc::http::server::EspHttpServer;
use esp_idf_svc::netif::EspNetifStack;
use esp_idf_svc::nvs::{EspDefaultNvs, EspNvs};
use esp_idf_svc::nvs_storage::EspNvsStorage;
use esp_idf_svc::sysloop::EspSysLoopStack;
use esp_idf_svc::wifi::EspWifi;
use esp_idf_sys::esp;
use log::{debug, error, info, warn};

use esp_idf_hal::adc;

// const SINGLE_PHASE_CURRENT_PIN: u8 = 35;
// const SINGLE_PHASE_VOLTAGE_PIN: u8 = 34;
// const THREE_PHASE_CURRENT_PINS: [u8; 3] = [32, 35, 34];
// const THREE_PHASE_VOLTAGE_PINS: [u8; 3] = [39, 36, 33];
// const LED_PIN: u8 = 14;
const DC_VOLTAGE: [u16; 3] = [1892; 3];
const DC_CURRENT: [u16; 3] = [1635; 3];
const CURRENT_SCALE: [f32; 3] = [102.0; 3]; //111.1;
const VOLTAGE_SCALE: [f32; 3] = [232.5; 3];
const MAX_SAMPLES: usize = 120;

const STORAGE_PARTITION_NAME: &str = "storage";
const STORAGE_NAMESPACE: &str = "st";

static ACCESS_TOKEN: String = String::new();
const GATEWAY_IP: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 1);

fn main() -> anyhow::Result<()> {
    esp_idf_sys::link_patches();

    // Bind the log crate to the ESP Logging facilities
    esp_idf_svc::log::EspLogger::initialize_default();

    // Initialize NVS storage
    let nvs = Arc::new(EspNvs::new(STORAGE_PARTITION_NAME)?);
    let mut storage = EspNvsStorage::new(nvs, STORAGE_NAMESPACE, true)?;

    let mut ap_ssid: String = String::new();
    let ap_password: &str = "12345678";

    configure_access_point_ssid(&mut ap_ssid)?;
    info!("Configured AP SSID as: {}.", ap_ssid);

    let _wifi = init_access_point(&ap_ssid, ap_password)?;
    info!("Initialized Wifi.");

    let _web_server = init_web_server()?;
    info!("Initialized Web Server.");

    // Initilize peripherals and pins
    let peripherals = Peripherals::take().unwrap();
    let pins = peripherals.pins;

    // Initilize ADC
    let mut single_phase_current_pin = pins.gpio35.into_analog_atten_11db()?;
    info!("Initialized ADC1 pin: GPIO35.");
    let mut single_phase_voltage_pin = pins.gpio34.into_analog_atten_11db()?;
    info!("Initialized ADC1 pin: GPIO34.");
    let mut powered_adc1 = adc::PoweredAdc::new(
        peripherals.adc1,
        adc::config::Config::new().calibration(false),
    )?;
    info!("Initialized ADC1.");

    //
    let ac_phase = 1;
    let mut current_samples = [0.0; MAX_SAMPLES];
    let mut voltage_samples = [0.0; MAX_SAMPLES];
    let mut sample_count = 0;

    loop {
        // Read current and voltage values, calibrate them
        // and add them to the samples array
        let raw_current_reading =
            powered_adc1.read(&mut single_phase_current_pin).unwrap() - DC_CURRENT[ac_phase];
        let current_reading =
            CURRENT_SCALE[ac_phase] * ((raw_current_reading as f32 * 3.3) / 4095.0);

        let raw_voltage_reading =
            powered_adc1.read(&mut single_phase_voltage_pin).unwrap() - DC_VOLTAGE[ac_phase];
        let voltage_reading =
            VOLTAGE_SCALE[ac_phase] * ((raw_voltage_reading as f32 * 3.3) / 4095.0);

        current_samples[sample_count] = current_reading;
        voltage_samples[sample_count] = voltage_reading;

        sample_count += 1;

        info!("Current: {}", current_reading);
        info!("Voltage: {}", voltage_reading);

        if sample_count >= MAX_SAMPLES {
            let current_rms = calc_rms(&current_samples, sample_count);
            let voltage_rms = calc_rms(&voltage_samples, sample_count);
            sample_count = 0;
            info!("Current RMS : {}", current_rms);
            info!("Voltage RMS : {}", voltage_rms);
        }

        sleep(Duration::from_millis(1000));
    }
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
fn init_access_point(ssid: &str, password: &str) -> anyhow::Result<Box<EspWifi>> {
    let netif_stack = Arc::new(EspNetifStack::new()?);
    let sys_loop_stack = Arc::new(EspSysLoopStack::new()?);
    let default_nvs = Arc::new(EspDefaultNvs::new()?);

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

    server.handle_get("/time", |req, mut res| {
        println!("{:#?}", req.query_string());
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
