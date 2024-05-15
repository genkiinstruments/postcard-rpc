use {
    wasm_bindgen::prelude::*,
    wasm_bindgen_futures::{JsFuture, spawn_local},
    js_sys::{Reflect, Array, Promise, Uint8Array},
    web_sys::{UsbDevice, UsbDeviceRequestOptions, UsbInTransferResult},
    log::{Level, info, warn, debug},
    tokio::sync::mpsc,
    std::sync::Arc,
    crate::client::WorkbookClient,
};

const VENDOR_ID: u16 = 0x16c0;
/// The Bulk Out Endpoint (0x00 | 0x01): Out EP 1
const BULK_OUT_EP: u8 = 0x01;
/// The Bulk In Endpoint (0x80 | 0x01): In EP 1
const BULK_IN_EP: u8 = 0x81;
/// The size in bytes of the largest possible IN transfer
const MAX_TRANSFER_SIZE: usize = 1024;

// Note: Has to be called as part of a user action in order for permissions dialog to show...
#[wasm_bindgen]
pub async fn open_usb_device() -> Result<(), js_sys::Error> {
    let window = web_sys::window().unwrap();
    let navigator = window.navigator();
    let usb = navigator.usb();

    let devices: Array = JsFuture::from(Promise::resolve(&usb.get_devices())).await.map_or(Array::new(), |res| res.into());
    let matched: Vec<UsbDevice> = devices.iter()
        .map(|js_device: JsValue| -> UsbDevice { js_device.into() })
        .filter(|dev| dev.vendor_id() == VENDOR_ID)
        .collect();

    info!("found {} matched devices for VID 0x{:4x}", matched.len(), VENDOR_ID);

    let device: UsbDevice = match matched.len() {
        0 => {
            let filters = {
                let filters = Array::new();
                let obj = js_sys::Object::new();
                Reflect::set(
                    &obj,
                    &JsValue::from_str("vendorId"),
                    &JsValue::from(VENDOR_ID),
                ).unwrap();

                filters.push(&obj);
                filters
            };

            let opts = UsbDeviceRequestOptions::new(&filters);
            JsFuture::from(Promise::resolve(&usb.request_device(&opts))).await?.into()
        },
        _ => devices.get(0).into(),
    };

    info!("Open USB device: {}", device.product_name().unwrap_or("no name".to_string()));

    let _ = JsFuture::from(Promise::resolve(&device.open())).await?;
    let _ = JsFuture::from(Promise::resolve(&device.select_configuration(1))).await?;
    let _ = JsFuture::from(Promise::resolve(&device.claim_interface(0))).await?;

    let in_device = Arc::new(device);
    let out_device = in_device.clone();

    let (frames_out_tx, mut frames_out_rx) = mpsc::channel::<Vec<u8>>(16);
    let (frames_in_tx, frames_in_rx) = mpsc::channel::<Vec<u8>>(16);

    spawn_local(async move {
        loop {
            // NOTE: WebUSB doesn't seem to want the top bit set for IN endpoints...
            let res = JsFuture::from(Promise::resolve(&in_device.transfer_in(BULK_IN_EP & 0x7f, MAX_TRANSFER_SIZE as u32))).await;

            if let Ok(ok) = res {
                let transfer_result: UsbInTransferResult = ok.into();

                if let Some(data) = transfer_result.data() {
                    frames_in_tx.send(Uint8Array::new(&data.buffer()).to_vec()).await.unwrap();
                }
            }
        }
    });

    spawn_local(async move {
        while let Some(mut bytes) = frames_out_rx.recv().await {
            let _ = JsFuture::from(Promise::resolve(&out_device.transfer_out_with_u8_array(BULK_OUT_EP, &mut bytes))).await; // TODO: result...
        }
    });

    let client = WorkbookClient::new(frames_in_rx, frames_out_tx);

    for i in 0..10 {
        sleep(250.into()).await;
        debug!("Pinging with {i}... ");
        let res = client.ping(i).await.unwrap();
        debug!("got {res}!");
        assert_eq!(res, i);
    }

    Ok(())
}

#[wasm_bindgen(module = "/sleep.js")]
extern "C" {
    async fn sleep(interval_ms: JsValue);
}

#[wasm_bindgen(start)]
async fn start() {
    console_log::init_with_level(Level::Debug).expect("Failed to initialize log");
    info!("Rust init");
}
