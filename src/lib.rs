#![no_std]
#![deny(warnings)]

use core::panic::PanicInfo;
use core::fmt::Write;
use core::mem::size_of;
use cfg_if::cfg_if;

static mut PANIC_LED_BLINKER: Option<fn ()> = None;

pub fn set_panic_led_blinker(blinker: fn ()) {
    unsafe {
        PANIC_LED_BLINKER = Some(blinker);
    }
}

struct DumbCursor<'a> {
    pub buf: &'a mut[u8],
    pub idx: usize
}

impl<'a> Write for DumbCursor<'a> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let s = s.as_bytes();
        let bytes_left = self.buf.len() - self.idx;
        let to_copy = if bytes_left >= s.len() { s.len() } else { bytes_left };
        unsafe {
            core::ptr::copy_nonoverlapping(
                s.as_ptr(),
                self.buf.as_mut_ptr().offset(self.idx as isize),
                to_copy
            );
        }
        self.idx += to_copy;
        Ok(())
    }
}

extern "C" {
    pub static mut _panic_info_ram_start: u8;
    pub static mut _panic_info_ram_end: u8;
}

pub fn ram_log_slice() -> &'static mut[u8] {
    unsafe {
        let panic_info_ram_start = &mut _panic_info_ram_start as *mut u8;
        let panic_info_ram_end = &mut _panic_info_ram_end as *mut u8;
        let panic_info_ram_length: usize = panic_info_ram_end as usize - panic_info_ram_start as usize;
        core::slice::from_raw_parts_mut(panic_info_ram_start, panic_info_ram_length)
    }
}

/// RAM layout: PanicInfoRam struct | filename (0 or more bytes) | message
#[derive(Default, Debug, Copy, Clone)]
pub struct PanicInfoMeta {
    pub filename_len: u8,
    pub line: u32,
    pub column: u32,
    pub message_len: u16,
    pub xor: u8
}

impl PanicInfoMeta {
    pub fn detect_and_reset() -> Option<Self> {
        let panic_info_ram = ram_log_slice();
        unsafe {
            let panic_info_meta = *(panic_info_ram.as_mut_ptr() as *const PanicInfoMeta);
            let mut xor = 0;
            for i in size_of::<PanicInfoMeta>()..panic_info_ram.len() {
                xor = xor ^ panic_info_ram.get_unchecked(i);
            }
            if xor == panic_info_meta.xor {
                core::ptr::write_bytes(panic_info_ram.as_mut_ptr(), 0, size_of::<PanicInfoMeta>());
                Some(panic_info_meta)
            } else {
                None
            }
        }
    }

    pub fn filename(&self) -> &'static str {
        unsafe {
            let panic_info_ram = ram_log_slice();
            let panic_filename_start = panic_info_ram.as_ptr().offset(size_of::<PanicInfoMeta>() as isize);
            let panic_filename = core::slice::from_raw_parts(panic_filename_start, self.filename_len as usize);
            core::str::from_utf8_unchecked(panic_filename)
        }
    }

    pub fn message(&self) -> &'static str {
        unsafe {
            let panic_info_ram = ram_log_slice();
            let panic_message_start = panic_info_ram.as_ptr().offset((size_of::<PanicInfoMeta>() + self.filename_len as usize) as isize);
            let panic_message = core::slice::from_raw_parts(panic_message_start, self.message_len as usize);
            core::str::from_utf8_unchecked(panic_message)
        }
    }
}

#[inline(never)]
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Format panic message into PANIC_INFO_RAM region
    unsafe {
        let panic_info_ram = ram_log_slice();

        let mut panic_info_meta = PanicInfoMeta::default();
        match info.location() {
            Some(l) => {
                let filename_len = if l.file().len() > 255 { 255 } else { l.file().len() as u8 };
                panic_info_meta.filename_len = filename_len;
                panic_info_meta.line = l.line();
                panic_info_meta.column = l.column();
                core::ptr::copy_nonoverlapping(
                    l.file() as *const _ as *mut u8,
                    panic_info_ram.as_mut_ptr().offset(size_of::<PanicInfoMeta>() as isize),
                    filename_len as usize
                );
            },
            None => {}
        }
        cfg_if! {
            if #[cfg(not(feature = "minimal"))] {
                let message_start_idx = size_of::<PanicInfoMeta>() + panic_info_meta.filename_len as usize;
                let mut cursor = DumbCursor {
                    buf: core::slice::from_raw_parts_mut(
                        panic_info_ram.as_mut_ptr().offset(message_start_idx as isize),
                        panic_info_ram.len() - message_start_idx
                    ),
                    idx: 0
                };
                let _ = write!(cursor, "{}", info);
                panic_info_meta.message_len = cursor.idx as u16;
            }
        }

        for i in size_of::<PanicInfoMeta>()..panic_info_ram.len() {
            panic_info_meta.xor = panic_info_meta.xor ^ *panic_info_ram.get_unchecked(i);
        }

        core::ptr::copy_nonoverlapping(
            &panic_info_meta as *const _ as *mut u8,
            panic_info_ram.as_mut_ptr(),
            core::mem::size_of::<PanicInfoMeta>()
        );
        match PANIC_LED_BLINKER {
            Some(blinker) => blinker(),
            None => {}
        }
    }
    cortex_m::peripheral::SCB::sys_reset(); // -> !
}