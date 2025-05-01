use std::ptr::null_mut;

use esp_idf_svc::{espnow::{EspNow, PeerInfo}, eventloop::EspSystemEventLoop, hal::{
    delay::FreeRtos, i2c::{I2cConfig, I2cDriver}, peripherals::Peripherals, prelude::*, reset::restart, sys::wifi_interface_t_WIFI_IF_STA, gpio::PinDriver
}, nvs::EspDefaultNvsPartition, wifi::{ClientConfiguration, Configuration, EspWifi}};

use as5600::{status::Status, As5600};

const ANGLE_VALUE_MAX: u16 = 0b111111111111; // AS5600 provides 12 bit precision
const ENCODER_STEPS: u8 = 16; // determines how many "clicks" the encoder has
const ANGLE_PER_STEP: u16 = ANGLE_VALUE_MAX / ENCODER_STEPS as u16;

// Replace with the MAC address of your receiver (LCD)! (see serial output of the receiver)
const MAC_ADDR_RECEIVER: [u8; 6] = [0x58, 0xBF, 0x25, 0x9D, 0xF5, 0x70];

#[derive(bytemuck::NoUninit, Clone, Copy)]
#[repr(C)]
struct ScrollData { // this has to perfectly match with the struct in the receiver's code!!
    rotation: i8, // positive -> clockwise steps, negative -> counter-clockwise steps
    buttons: u8, // each bit one button
}


fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let peripherals = match Peripherals::take() {
        Ok(periphs) => periphs,
        Err(error) => {
            log::error!("Peripherals failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    // ### WiFi/ESP-NOW setup ###
    let sys_loop = match EspSystemEventLoop::take() {
        Ok(sysloop) => sysloop,
        Err(error) => {
            log::error!("ESP sys loop failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };
    
    let nvs_part = match EspDefaultNvsPartition::take() {
        Ok(part) => part,
        Err(error) => {
            log::error!("ESP NVS failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    let mut esp_wifi = match EspWifi::new(peripherals.modem, sys_loop, Some(nvs_part)) {
        Ok(wifi) => wifi,
        Err(error) => {
            log::error!("ESP WiFi failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    let wifi_config = ClientConfiguration::default();

    match esp_wifi.set_configuration(&Configuration::Client(wifi_config)) {
        Ok(_res) => {},
        Err(error) => {
            log::error!("ESP WiFi config failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    match esp_wifi.start() {
        Ok(_res) => {},
        Err(error) => {
            log::error!("ESP WiFi start failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    while !esp_wifi.is_started().unwrap_or(false) {
        log::info!("Waiting for WiFi to start...");
        FreeRtos::delay_ms(100u32);
    }

    let esp_now = match EspNow::take() {
        Ok(now) => now,
        Err(error) => {
            log::error!("ESP-NOW failed! Restarting in 1s...");
            log::error!("{}", error);
            FreeRtos::delay_ms(1000u32);
            restart();
        }
    };

    let peer_info = PeerInfo {
        peer_addr: MAC_ADDR_RECEIVER,
        channel: 1,
        ifidx: wifi_interface_t_WIFI_IF_STA,
        encrypt: false,
        lmk: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        priv_: null_mut(),
    };

    if esp_now.add_peer(peer_info).is_err() {
        log::error!("Failed to add peer! Restarting in 1s...");
        FreeRtos::delay_ms(1000u32);
        restart();
    }


    // ### Buttons setup ###
    let pause_pin = PinDriver::input(peripherals.pins.gpio21).unwrap();
    let next_song_pin = PinDriver::input(peripherals.pins.gpio22).unwrap();
    let prev_song_pin = PinDriver::input(peripherals.pins.gpio32).unwrap();
    let click_pin = PinDriver::input(peripherals.pins.gpio33).unwrap();
    let right_pin = PinDriver::input(peripherals.pins.gpio34).unwrap();
    let left_pin = PinDriver::input(peripherals.pins.gpio35).unwrap();


    // ### AS5600 setup ###
    let sda = peripherals.pins.gpio18;
    let scl = peripherals.pins.gpio16;
    let peripherals_i2c = peripherals.i2c0;

    let config = I2cConfig::new().baudrate(100.kHz().into());

    let i2c = match I2cDriver::new(peripherals_i2c, sda, scl, &config) {
        Ok(driver) => driver,
        Err(error) => {
            log::error!("I2C driver failed! Restarting in 1s...");
            log::error!("{}", error);
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


    // ### loop ##
    let mut encoder_position_last: i8 = 0;

    let mut scroll_data = ScrollData {
        rotation: 0,
        buttons: 0,
    };

    let mut send = false;
    let mut last_send: u8 = 0; // tracks how many loop iteration a send had to wait

    loop {
        // only evaluate if sensor can actually read properly
        if as5600.magnet_status().unwrap_or(Status::MagnetLow) != Status::MagnetLow && as5600.magnet_status().unwrap_or(Status::MagnetHigh) != Status::MagnetHigh {
            let new_angle = as5600.angle().unwrap_or(0);
            let encoder_position = ((new_angle / ANGLE_PER_STEP) % ENCODER_STEPS as u16) as i8;
            log::info!("Position: {} / {}", encoder_position, ENCODER_STEPS - 1);

            // positive -> clockwise rotation, negative -> counter-clockwise rotation
            let mut difference = encoder_position - encoder_position_last;

            // take care of edge case where angle flips over (more or less limits rotation speed for accurate readings)
            if difference.unsigned_abs() > ENCODER_STEPS / 2 {
                if difference.is_positive() {
                    difference -= ENCODER_STEPS as i8;
                } else {
                    difference += ENCODER_STEPS as i8;
                }
            }

            // only send if something happened
            if difference != 0 {
                scroll_data.rotation += difference;
                send = true;

                log::info!("Rotated {} steps {}clockwise", difference.abs(), match difference > 0 {true => "", false => "counter-"});
            }

            encoder_position_last = encoder_position;
        }

        // check buttons and set bits accordingly
        if pause_pin.is_low() {
            scroll_data.buttons &= 1u8;
            send = true;
        }

        if next_song_pin.is_low() {
            scroll_data.buttons &= 1u8 << 1;
            send = true;
        }

        if prev_song_pin.is_low() {
            scroll_data.buttons &= 1u8 << 2;
            send = true;
        }

        if click_pin.is_low() {
            scroll_data.buttons &= 1u8 << 3;
            send = true;
        }

        if right_pin.is_low() {
            scroll_data.buttons &= 1u8 << 4;
            send = true;
        }

        if left_pin.is_low() {
            scroll_data.buttons &= 1u8 << 5;
            send = true;
        }


        if send && last_send >= 3 { // last_send to group events and avoid "bloat"
            let bytes = bytemuck::bytes_of(&scroll_data);

            match esp_now.send(MAC_ADDR_RECEIVER, bytes) {
                Ok(_res) => {
                    log::info!("Sent ESP-NOW message");
                },
                Err(error) => {
                    log::error!("ESP-NOW send error: {}", error);
                }
            };

            scroll_data.rotation = 0;
            scroll_data.buttons = 0;
            send = false;
            last_send = 0;
        }


        FreeRtos::delay_ms(50u32);
        if send { // only increment if there is actually something to send
            last_send += 1;
        }
    }
}
