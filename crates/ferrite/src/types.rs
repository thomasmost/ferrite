//! Safe value types: re-exports of the `#[repr(C)]` SDK value types and
//! their const constructors. They cross the FFI boundary untranslated.

pub use crate::sys::{
    GColor, GColor8, GColorClear, GColorBlack, GColorWhite, GColorFromHEX,
    GColorFromRGB, GColorFromRGBA, GPoint, GRect, GSize,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_from_rgb_packs_two_bit_channels() {
        // argb bit layout: a[7:6] r[5:4] g[3:2] b[1:0]
        assert_eq!(unsafe { GColorFromRGB(255, 0, 0).argb }, 0b1111_0000);
        assert_eq!(unsafe { GColorFromRGB(0, 255, 0).argb }, 0b1100_1100);
        assert_eq!(unsafe { GColorFromRGB(0, 0, 255).argb }, 0b1100_0011);
        assert_eq!(unsafe { GColorFromHEX(0xFFFFFF).argb }, 0b1111_1111);
        assert_eq!(unsafe { GColorFromRGBA(0, 0, 0, 0).argb }, 0b0000_0000);
    }

    #[test]
    fn grect_constructor_and_layout() {
        let r = GRect(1, 2, 3, 4);
        assert_eq!(r.origin.x, 1);
        assert_eq!(r.origin.y, 2);
        assert_eq!(r.size.w, 3);
        assert_eq!(r.size.h, 4);
        assert_eq!(core::mem::size_of::<GRect>(), 8);
    }
}
