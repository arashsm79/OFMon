use std::ptr;

use anyhow::Result;
use embedded_svc::io::Read;
use esp_idf_svc::http::server::EspHttpRequest;
use esp_idf_sys::{self as _, c_types::c_void, esp};
use log::*;

pub fn ota_update_from_reader(mut reader: &mut EspHttpRequest) -> anyhow::Result<()> {
    info!("Updating firmware");
    let next_partition = unsafe { esp_idf_sys::esp_ota_get_next_update_partition(ptr::null()) };
    let mut ota_handle = esp_idf_sys::esp_ota_handle_t::default();
    esp!(unsafe { esp_idf_sys::esp_ota_begin(next_partition, 0, &mut ota_handle) })?;
    let mut buf = [0; 1024];
    let mut size = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size += n;
        info!("read {} bytes so far", size);

        esp!(unsafe {
            esp_idf_sys::esp_ota_write(ota_handle, buf.as_ptr() as *const c_void, n as u32)
        })?;
    }

    esp!(unsafe { esp_idf_sys::esp_ota_end(ota_handle) })?;
    esp!(unsafe { esp_idf_sys::esp_ota_set_boot_partition(next_partition) })?;
    info!("Update completed, restarting.");
    unsafe {
        esp_idf_sys::esp_restart();
    }

    Ok(())
}

pub fn first_run_validate() -> Result<()> {
    info!("Validating image.");
    unsafe {
        let cur_partition = esp_idf_sys::esp_ota_get_running_partition();
        let mut ota_state: esp_idf_sys::esp_ota_img_states_t = 0;
        if let Ok(()) = esp!(esp_idf_sys::esp_ota_get_state_partition(
            cur_partition,
            &mut ota_state
        )) {
            if ota_state == esp_idf_sys::esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY {
                // Validate image
                esp!(esp_idf_sys::esp_ota_mark_app_valid_cancel_rollback())?;
                info!("Image validated.");
            }
        }
    }

    Ok(())
}
