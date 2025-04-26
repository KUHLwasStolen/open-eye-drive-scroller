use esp_idf_svc::hal::{
    delay::FreeRtos,
    i2c::{I2cConfig, I2cDriver},
    peripherals::Peripherals,
    prelude::*,
    reset::restart,
};

use as5600::{status::Status, As5600};


const ANGLE_VALUE_MAX: u16 = 0b111111111111; // AS5600 provides 12 bit precision
const ENCODER_STEPS: u8 = 16;
const ANGLE_PER_STEP: u16 = ANGLE_VALUE_MAX / ENCODER_STEPS as u16;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = match Peripherals::take() {
        Ok(periphs) => periphs,
        Err(error) => {
            log::error!("Peripherals failed! Restarting in 1s...");
            log::error!("{:?}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    let sda = peripherals.pins.gpio18;
    let scl = peripherals.pins.gpio16;
    let peripherals_i2c = peripherals.i2c0;

    let config = I2cConfig::new().baudrate(100.kHz().into());

    let i2c = match I2cDriver::new(peripherals_i2c, sda, scl, &config) {
        Ok(driver) => driver,
        Err(error) => {
            log::error!("I2C driver failed! Restarting in 1s...");
            log::error!("{:?}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };
    
    let mut as5600 = As5600::new(i2c);

    match as5600.config() {
        Ok(as_config) => {
            log::info!("AS5600 Config:\n{:?}", as_config);
        },
        Err(error) => {
            log::warn!("Could not retrieve AS5600 config...");
            log::warn!("{:?}", error);
        }
    };

    let mut encoder_position_last: i16 = 0;

    loop {
        // only evaluate if sensor can actually read properly
        if as5600.magnet_status().unwrap_or(Status::MagnetLow) != Status::MagnetLow && as5600.magnet_status().unwrap_or(Status::MagnetHigh) != Status::MagnetHigh {
            let new_angle = as5600.angle().unwrap_or(0);
            let encoder_position = ((new_angle / ANGLE_PER_STEP) % ENCODER_STEPS as u16) as i16;
            log::info!("Position: {} / {}", encoder_position, ENCODER_STEPS - 1);

            let mut difference = encoder_position - encoder_position_last;

            // take care of edge case where angle flips over
            if difference.abs() > ENCODER_STEPS as i16 / 2 {
                if difference.is_positive() {
                    difference -= ENCODER_STEPS as i16;
                } else {
                    difference += ENCODER_STEPS as i16;
                }
            }

            // positive -> clockwise rotation, negative -> counter-clockwise rotation
            if difference > 0 {
                log::info!("Moved {} step(s) clockwise", difference.abs());
            } else if difference < 0 {
                log::info!("Moved {} step(s) counter-clockwise", difference.abs());
            }

            encoder_position_last = encoder_position;
        }

        FreeRtos::delay_ms(200u32);
    }
}
