use std::{collections::HashMap, sync::Arc};

use cross_usb::{
    self,
    DeviceFilter,
    usb::{UsbDevice, UsbDescriptor, UsbInterface},
};
use postcard::experimental::schema::Schema;
use serde::de::DeserializeOwned;
use tokio::sync::{
    mpsc::{error::TrySendError, Receiver, Sender},
    Mutex,
};

use crate::{
    headered::extract_header_from_bytes,
    host_client::{HostClient, HostContext, ProcessError, RpcFrame, SubInfo, WireContext},
    Key,
};

// TODO: These should all be configurable, PRs welcome

/// The Bulk Out Endpoint (0x00 | 0x01): Out EP 1
pub(crate) const BULK_OUT_EP: u8 = 0x01;
/// The Bulk In Endpoint (0x80 | 0x01): In EP 1
pub(crate) const BULK_IN_EP: u8 = 0x81;
/// The size in bytes of the largest possible IN transfer
pub(crate) const MAX_TRANSFER_SIZE: usize = 1024;
// /// How many in-flight requests at once - allows nusb to keep pulling frames
// /// even if we haven't processed them host-side yet.
// pub(crate) const IN_FLIGHT_REQS: usize = 4;
// /// How many consecutive IN errors will we try to recover from before giving up?
// pub(crate) const MAX_STALL_RETRIES: usize = 10;

struct UsbCtx {
    sub_map: Mutex<HashMap<Key, Sender<RpcFrame>>>,
}

async fn cross_usb_worker(filters: Vec<DeviceFilter>, ctx: WireContext) -> Result<(), String> {
    let x = cross_usb::get_device(filters).await
        .map_err(|e| format!("Error listing devices: {e:?}"))?;

    let dev = x
        .open().await
        .map_err(|e| format!("Failed opening device: {e:?}"))?;

    let interface = dev
        .open_interface(0).await
        .map_err(|e| format!("Failed claiming interface: {e:?}"))?;

    // let boq = interface.bulk_out_queue(BULK_OUT_EP);
    // let biq = interface.bulk_in_queue(BULK_IN_EP);

    let WireContext {
        outgoing,
        incoming,
        new_subs,
    } = ctx;

    let usb_ctx = Arc::new(UsbCtx {
        sub_map: Mutex::new(HashMap::new()),
    });

    let ai = Arc::new(interface);

    tokio::task::spawn(out_worker(ai.clone(), outgoing));
    tokio::task::spawn(in_worker(ai.clone(), incoming, usb_ctx.clone()));
    tokio::task::spawn(sub_worker(new_subs, usb_ctx));

    Ok(())
}

/// Output worker, feeding frames to nusb.
///
/// TODO: We could maybe do something clever and have multiple "in flight" requests
/// with nusb, instead of doing it ~serially. If you are noticing degraded OUT endpoint
/// bandwidth (e.g. PC to USB device), lemme know and we can look at this.
async fn out_worker(iface: Arc<cross_usb::Interface>, mut rec: Receiver<RpcFrame>) {
    loop {
        let Some(msg) = rec.recv().await else {
            tracing::info!("Receiver Closed, existing out_worker");
            return;
        };

        let send_res = iface.bulk_out(BULK_OUT_EP, &msg.to_bytes()).await;
        if let Err(e) = send_res {
            tracing::error!("Output Queue Error: {e:?}, exiting");
            rec.close();
            return;
        }
    }
}

/// Input worker, getting frames from nusb
async fn in_worker(iface: Arc<cross_usb::Interface>, ctxt: Arc<HostContext>, usb_ctx: Arc<UsbCtx>) {
    let mut consecutive_errs = 0;

    loop {
        let res = iface.bulk_in(BULK_IN_EP, MAX_TRANSFER_SIZE).await;

        if let Err(e) = res {
            consecutive_errs += 1;

            tracing::error!("In Worker error: {e:?}, consecutive: {consecutive_errs:?}");
            continue;
        }

        let Ok(data) = res else {
            tracing::warn!("Failed: {:?}", res);
            continue;
        };

        let Ok((hdr, body)) = extract_header_from_bytes(&data) else {
            tracing::warn!("Header decode error!");
            continue;
        };

        // If we get a good decode, clear the error flag
        if consecutive_errs != 0 {
            tracing::info!("Clearing consecutive error counter after good header decode");
            consecutive_errs = 0;
        }

        let mut handled = false;

        {
            let mut sg = usb_ctx.sub_map.lock().await;
            let key = hdr.key;

            // Remove if sending fails
            let rem = if let Some(m) = sg.get(&key) {
                handled = true;
                let frame = RpcFrame {
                    header: hdr.clone(),
                    body: body.to_vec(),
                };
                let res = m.try_send(frame);

                match res {
                    Ok(()) => {
                        tracing::debug!("Handled message via subscription");
                        false
                    }
                    Err(TrySendError::Full(_)) => {
                        tracing::error!("Subscription channel full! Message dropped.");
                        false
                    }
                    Err(TrySendError::Closed(_)) => true,
                }
            } else {
                false
            };

            if rem {
                tracing::debug!("Dropping subscription");
                sg.remove(&key);
            }
        }

        if handled {
            continue;
        }

        let frame = RpcFrame {
            header: hdr,
            body: body.to_vec(),
        };
        match ctxt.process_did_wake(frame) {
            Ok(true) => tracing::debug!("Handled message via map"),
            Ok(false) => tracing::debug!("Message not handled"),
            Err(ProcessError::Closed) => {
                tracing::warn!("Got process error, quitting");
                return;
            }
        }
    }
}

async fn sub_worker(mut new_subs: Receiver<SubInfo>, usb_ctx: Arc<UsbCtx>) {
    while let Some(sub) = new_subs.recv().await {
        let mut sg = usb_ctx.sub_map.lock().await;
        if let Some(_old) = sg.insert(sub.key, sub.tx) {
            tracing::warn!("Replacing old subscription for {:?}", sub.key);
        }
    }
}

impl<WireErr> HostClient<WireErr>
where
    WireErr: DeserializeOwned + Schema,
{
    pub async fn try_new_cross_usb(
        filters: Vec<DeviceFilter>,
        err_uri_path: &str,
        outgoing_depth: usize,
    ) -> Result<Self, String> {
        let (me, wire) = Self::new_manual(err_uri_path, outgoing_depth);
        cross_usb_worker(filters, wire).await?;
        Ok(me)
    }

    pub async fn new_cross_usb (
        filters: Vec<DeviceFilter>,
        err_uri_path: &str,
        outgoing_depth: usize,
    ) -> Self {
        Self::try_new_cross_usb(filters, err_uri_path, outgoing_depth).await
            .expect("should have found cross_usb device")
    }
}
