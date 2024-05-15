import { open_usb_device } from './pkg';

document.querySelector('#button').addEventListener('click', () => {
    open_usb_device();
});
