#![no_std]

use {
    defmt_rtt as _,
    embassy_nrf::{
        bind_interrupts,
        peripherals::{
            USBD,
        },
        usb,
    },
    panic_probe as _,
};

use embassy_time as _;

bind_interrupts!(pub struct Irqs {
    // ADC_IRQ_FIFO => adc::InterruptHandler;
    USBD => usb::InterruptHandler<USBD>;
    POWER_CLOCK => usb::vbus_detect::InterruptHandler;

});

pub const NUM_SMARTLEDS: usize = 24;
