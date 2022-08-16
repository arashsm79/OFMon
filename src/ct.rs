use crate::{now, set_system_time, ACCESS_TOKEN_SIZE, MAX_TIME_STORAGE_SIZE};
use std::collections::HashSet;
use std::io::{Read, Seek, Write};

use std::{fs, ops};

use embedded_hal_0_2_7::adc::OneShot;

use embedded_svc::io::Write as SvcWrite;
use esp_idf_hal::adc::{Atten11dB, PoweredAdc, ADC1};
use esp_idf_hal::gpio::{Gpio34, Gpio35, Pins};
use esp_idf_svc::http::server::EspHttpResponseWrite;
use esp_idf_sys::settimeofday;

use crate::{
    utils::*, AC_PHASE, CT_READING_SIZE, MAX_MV_ATTEN_11, MAX_SHARD_SIZE, NOISE_THRESHOLD,
    SAVE_PERIOD_TIMEOUT, SUPPLY_VOLTAGE,
};

use anyhow::bail;
use cstr::cstr;
#[allow(unused_imports)]
use log::{debug, error, info, warn};
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

pub struct CT {
    id: u16,
    current_pin: CurrentPin,
    voltage_pin: VoltagePin,
    pub reading: CTReading,
}

#[derive(Debug)]
pub struct CTReading {
    real_power: f32,
    apparent_power: f32,
    i_rms: f32,
    v_rms: f32,
    kwh: f32,
    timestamp: u64,
}

pub struct CTStorage {
    pub readings_shard_counter: i32,
    pub readings_shards: HashSet<i32>,
}

impl CTStorage {
    pub(crate) fn new() -> Self {
        CTStorage {
            readings_shard_counter: 1,
            readings_shards: HashSet::new(),
        }
    }

    // Whenever the esp boots, it restores the previously set RTC and stores that RTC in a log.
    pub(crate) fn log_powerloss(&mut self) -> anyhow::Result<()> {
        if let Ok(mut file) = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open("/littlefs/powerloss_log")
        {
            file.write_all(&now().as_millis().to_le_bytes())?;
        }
        Ok(())
    }

    // Dump the powerloss log into the given writer and after that delete the log.
    pub(crate) fn send_and_clear_powerloss_log(
        &mut self,
        writer: &mut EspHttpResponseWrite,
    ) -> anyhow::Result<()> {
        let mut powerloss_log_sent = false;
        // open the log file and send data. If no log is available an empty response is sent.
        if let Ok(mut file) = fs::OpenOptions::new()
            .write(true)
            .read(true)
            .open("/littlefs/powerloss_log")
        {
            let mut buf = [0_u8; std::mem::size_of::<u128>() * 5];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                writer.write_all(&buf)?;
            }
            writer.flush()?;
            powerloss_log_sent = true;
        }
        if powerloss_log_sent {
            std::fs::remove_file("/littlefs/powerloss_log")?;
        }
        Ok(())
    }

    /// Find the newest readings shard id
    ///
    /// under "/littlefs/ct_readings" files are saved with a number as their filename.
    /// here we iterate through all of them and find the newest file (the one with higher number as
    /// its filename). This is the file that we will be appending new data to.
    pub(crate) fn find_newest_readings_shard_num(&mut self) -> anyhow::Result<()> {
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
    pub(crate) fn save_to_storage(&mut self, cts: &[CT; AC_PHASE]) -> anyhow::Result<()> {
        // check whether the selected shard has enough size. if it doesn't create a new shard
        if ((MAX_SHARD_SIZE
            - fs::metadata(format!(
                "/littlefs/ct_readings/{}",
                self.readings_shard_counter
            ))?
            .len()) as usize)
            < CT_READING_SIZE
        {
            self.readings_shard_counter += 1;
            self.readings_shards.insert(self.readings_shard_counter);
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(format!(
                "/littlefs/ct_readings/{}",
                self.readings_shard_counter
            ))?;
        info!(
            "Opened {} for writing.",
            format!("/littlefs/ct_readings/{}", self.readings_shard_counter)
        );

        // Append the readings for each CT at the end of the file
        for ct in cts {
            let buf = CTStorage::ct_reading_to_le_bytes(ct)?;
            file.write_all(&buf)?;
            info!("Wrote reading: {:?}", ct.reading)
        }
        file.flush()?;
        info!("Flushed readings to storage.");
        Ok(())
    }

    // Retrieve the latest time from storage and update RTC
    pub(crate) fn update_system_time(&mut self) -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/littlefs/time")?;
        file.seek(std::io::SeekFrom::End(-(std::mem::size_of::<u64>() as i64)))?;
        let mut time_buf = [12_u8; 8];
        file.read_exact(&mut time_buf)?;
        let time = u64::from_le_bytes(time_buf);
        set_system_time(time)?;
        Ok(())
    }

    // Store the given time to storage
    pub(crate) fn store_time(&mut self, time: u64) -> anyhow::Result<()> {
        let mut file = if ((MAX_TIME_STORAGE_SIZE - fs::metadata("/littlefs/time/{}")?.len())
            as usize)
            < std::mem::size_of::<u64>()
        {
            // If the file is full, create a new one overwriting the previous file.
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open("/littlefs/time")?
        } else {
            fs::OpenOptions::new()
                .write(true)
                .create(true)
                .append(true)
                .open("/littlefs/time")?
        };

        file.write_all(&time.to_le_bytes())?;
        Ok(())
    }

    // Retrieve the latest token from storage
    pub(crate) fn retrieve_token(&mut self) -> anyhow::Result<[u8; ACCESS_TOKEN_SIZE]> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/littlefs/time")?;
        let mut token = [0_u8; ACCESS_TOKEN_SIZE];
        file.read_exact(&mut token)?;
        Ok(token)
    }

    // Store the given token to storage
    pub(crate) fn store_token(&mut self, token: &[u8]) -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open("/littlefs/token")?;
        file.write_all(token)?;
        Ok(())
    }

    // Send reading shards one by one into this writer.
    // before deleting a shard, we make sure that he have flushed thr writer.
    pub(crate) fn send_readings_shards(
        &mut self,
        writer: &mut EspHttpResponseWrite,
    ) -> anyhow::Result<()> {
        let sorted_shard_ids = self.readings_shards.iter().copied().collect::<Vec<i32>>();
        // a fixed size buffer to avoid stack overflow
        let mut buf = [0_u8; CT_READING_SIZE * 5];
        for shard_id in sorted_shard_ids {
            let mut sent_shard = false;
            if let Ok(mut file) = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .append(true)
                .open(format!("/littlefs/ct_readings/{}", shard_id))
            {
                loop {
                    let n = file.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    writer.write_all(&buf)?;
                }
                writer.flush()?;
                info!(
                    "Sent shard {}",
                    format!("/littlefs/ct_readings/{}", shard_id)
                );
                sent_shard = true;
            }
            // if shard was successfully sent, delete it from storage.
            if sent_shard {
                fs::remove_file(format!("/littlefs/ct_readings/{}", shard_id))?;
                info!(
                    "Deleted shard {}",
                    format!("/littlefs/ct_readings/{}", shard_id)
                );
                self.readings_shards.remove(&shard_id);
                // if we have deleted the last shard, reset the counter
                if shard_id == self.readings_shard_counter {
                    self.readings_shard_counter = 1;
                }
            }
        }
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

impl CT {
    pub(crate) fn calculate_energy(
        &mut self,
        powered_adc1: &mut PoweredAdc<ADC1>,
        crossing: u32,
        timeout: std::time::Duration,
    ) -> anyhow::Result<()> {
        // Variables
        let mut cross_count = 0;
        let mut n_samples: u32 = 0;

        // Used for delay/phase compensation
        let mut filtered_v;
        let mut last_filtered_v = 0.0;
        let mut filtered_i;
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

    pub(crate) fn init(pins: Pins) -> anyhow::Result<[CT; AC_PHASE]> {
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

    pub(crate) fn reset(&mut self) {
        self.reading.reset();
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

impl CTReading {
    fn reset(&mut self) {
        self.i_rms = 0.0;
        self.v_rms = 0.0;
        self.real_power = 0.0;
        self.apparent_power = 0.0;
        self.kwh = 0.0;
        self.timestamp = 0;
    }
}
