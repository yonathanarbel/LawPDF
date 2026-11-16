//! A real Windows cursor, with the marker tip as its hot spot. Egui 0.33
//! exposes only named cursors, so the window subclass supplies our cursor for
//! client-area cursor messages while egui continues to manage every other one.
use super::*;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::Cell, ffi::c_void, ptr};

type Handle = *mut c_void;
type SubclassProc = unsafe extern "system" fn(Handle, u32, usize, isize, usize, usize) -> isize;

const SIZE: i32 = 32;
const WM_SETCURSOR: u32 = 0x0020;
const WM_REFRESH_MARKER_CURSOR: u32 = 0x8000 + 0x0c50;
const HTCLIENT: isize = 1;
const SUBCLASS_ID: usize = 0x4c_50_44_46;

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}
#[repr(C)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pixels_per_meter: i32,
    y_pixels_per_meter: i32,
    colors_used: u32,
    colors_important: u32,
}
#[repr(C)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 1],
}
#[repr(C)]
struct IconInfo {
    is_icon: i32,
    hotspot_x: u32,
    hotspot_y: u32,
    mask: Handle,
    color: Handle,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn CreateIconIndirect(info: *const IconInfo) -> Handle;
    fn DestroyCursor(cursor: Handle) -> i32;
    fn SetCursor(cursor: Handle) -> Handle;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn WindowFromPoint(point: Point) -> Handle;
    fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
}
#[link(name = "gdi32")]
unsafe extern "system" {
    fn CreateDIBSection(
        dc: Handle,
        info: *const BitmapInfo,
        usage: u32,
        pixels: *mut *mut c_void,
        section: Handle,
        offset: u32,
    ) -> Handle;
    fn CreateBitmap(width: i32, height: i32, planes: u32, bits: u32, data: *const c_void)
    -> Handle;
    fn DeleteObject(object: Handle) -> i32;
}
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(window: Handle, proc: Option<SubclassProc>, id: usize, data: usize)
    -> i32;
    fn RemoveWindowSubclass(window: Handle, proc: Option<SubclassProc>, id: usize) -> i32;
    fn DefSubclassProc(window: Handle, message: u32, wparam: usize, lparam: isize) -> isize;
}

thread_local! {
    // Both the subclass and egui run on the window's UI thread.
    static ACTIVE_CURSOR: Cell<Handle> = const { Cell::new(ptr::null_mut()) };
}

pub(in crate::app) struct NativeMarkerCursor {
    window: Handle,
    cursors: [Handle; MARKER_PRESETS.len()],
    selected: Option<usize>,
}

impl NativeMarkerCursor {
    pub(super) fn new(frame: &eframe::Frame) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = frame.window_handle().ok()?.as_raw() else {
            return None;
        };
        let window = handle.hwnd.get() as Handle;
        // The callback has no borrowed state and is removed when this owner drops.
        if unsafe { SetWindowSubclass(window, Some(window_proc), SUBCLASS_ID, 0) } == 0 {
            return None;
        }
        Some(Self {
            window,
            cursors: [ptr::null_mut(); MARKER_PRESETS.len()],
            selected: None,
        })
    }

    /// Returns false only when a native cursor could not be created.
    pub(super) fn set(&mut self, selected: Option<usize>) -> bool {
        if selected == self.selected {
            return true;
        }
        let cursor = if let Some(index) = selected {
            let Some(slot) = self.cursors.get_mut(index) else {
                return false;
            };
            if slot.is_null() {
                *slot = create_cursor(MARKER_PRESETS[index]);
            }
            if slot.is_null() {
                ACTIVE_CURSOR.with(|active| active.set(ptr::null_mut()));
                self.selected = None;
                return false;
            }
            *slot
        } else {
            ptr::null_mut()
        };
        self.selected = selected;
        ACTIVE_CURSOR.with(|active| active.set(cursor));
        // Apply after egui/winit has processed this frame's ordinary cursor.
        unsafe {
            PostMessageW(self.window, WM_REFRESH_MARKER_CURSOR, 0, 0);
        }
        true
    }
}

impl Drop for NativeMarkerCursor {
    fn drop(&mut self) {
        ACTIVE_CURSOR.with(|active| active.set(ptr::null_mut()));
        unsafe {
            RemoveWindowSubclass(self.window, Some(window_proc), SUBCLASS_ID);
            for cursor in self.cursors.into_iter().filter(|cursor| !cursor.is_null()) {
                DestroyCursor(cursor);
            }
        }
    }
}

unsafe fn pointer_is_over(window: Handle) -> bool {
    let mut point = Point { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut point) != 0 && WindowFromPoint(point) == window }
}

unsafe extern "system" fn window_proc(
    window: Handle,
    message: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    let active = ACTIVE_CURSOR.try_with(Cell::get).unwrap_or(ptr::null_mut());
    if message == WM_SETCURSOR
        && lparam & 0xffff == HTCLIENT
        && !active.is_null()
        && unsafe { pointer_is_over(window) }
    {
        unsafe {
            SetCursor(active);
        }
        return 1;
    }
    if message == WM_REFRESH_MARKER_CURSOR {
        if unsafe { pointer_is_over(window) } {
            if active.is_null() {
                // Restore the cursor winit chose for this part of the window.
                unsafe {
                    DefSubclassProc(window, WM_SETCURSOR, window as usize, HTCLIENT);
                }
            } else {
                unsafe {
                    SetCursor(active);
                }
            }
        }
        return 0;
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}

/// Rasterize a small angled highlighter into premultiplied BGRA, using 4x4
/// coverage samples for smooth edges. (3, 28) is the nib and the hot spot.
fn cursor_pixels(preset: MarkerPreset) -> Vec<u8> {
    let mut pixels = vec![0; (SIZE * SIZE * 4) as usize];
    let color = preset
        .color_rgb
        .map(|value| (value.clamp(0.0, 1.0) * 255.0) as u32);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut sums = [0_u32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let dx = x as f32 + (sx as f32 + 0.5) / 4.0 - 3.0;
                    let dy = y as f32 + (sy as f32 + 0.5) / 4.0 - 28.0;
                    let along = (dx - dy) * std::f32::consts::FRAC_1_SQRT_2;
                    let side = (dx + dy) * std::f32::consts::FRAC_1_SQRT_2;
                    let width = if along < 6.0 { along * 0.7 } else { 4.2 };
                    if (0.0..=29.0).contains(&along) && side.abs() <= width {
                        let fill = along >= 6.0 && along <= 28.0 && side.abs() <= 3.15;
                        let rgb = if fill { color } else { [48, 44, 42] };
                        sums[0] += rgb[2];
                        sums[1] += rgb[1];
                        sums[2] += rgb[0];
                        sums[3] += 255;
                    }
                }
            }
            let offset = ((y * SIZE + x) * 4) as usize;
            for channel in 0..4 {
                pixels[offset + channel] = (sums[channel] / 16) as u8;
            }
        }
    }
    pixels
}

fn create_cursor(preset: MarkerPreset) -> Handle {
    let info = BitmapInfo {
        header: BitmapInfoHeader {
            size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            width: SIZE,
            height: -SIZE,
            planes: 1,
            bit_count: 32,
            compression: 0,
            size_image: 0,
            x_pixels_per_meter: 0,
            y_pixels_per_meter: 0,
            colors_used: 0,
            colors_important: 0,
        },
        colors: [0],
    };
    let mut data = ptr::null_mut();
    unsafe {
        let color = CreateDIBSection(ptr::null_mut(), &info, 0, &mut data, ptr::null_mut(), 0);
        if color.is_null() {
            return ptr::null_mut();
        }
        if data.is_null() {
            DeleteObject(color);
            return ptr::null_mut();
        }
        let pixels = cursor_pixels(preset);
        ptr::copy_nonoverlapping(pixels.as_ptr(), data.cast::<u8>(), pixels.len());
        let mask_bits = [0_u8; (SIZE * SIZE / 8) as usize];
        let mask = CreateBitmap(SIZE, SIZE, 1, 1, mask_bits.as_ptr().cast());
        if mask.is_null() {
            DeleteObject(color);
            return ptr::null_mut();
        }
        let icon = IconInfo {
            is_icon: 0,
            hotspot_x: 3,
            hotspot_y: 28,
            mask,
            color,
        };
        let cursor = CreateIconIndirect(&icon);
        DeleteObject(mask);
        DeleteObject(color);
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_contains_selected_color_and_a_transparent_background() {
        let blue = cursor_pixels(MARKER_PRESETS[3]);
        let pink = cursor_pixels(MARKER_PRESETS[1]);
        assert_ne!(blue, pink);
        assert_eq!(&blue[0..4], &[0, 0, 0, 0]);
        assert!(
            blue.chunks_exact(4)
                .any(|pixel| pixel[3] == 255 && pixel[0] > pixel[2])
        );
    }
}
