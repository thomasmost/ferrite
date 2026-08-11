//! Safe value types: re-exports of the `#[repr(C)]` SDK value types and
//! their const constructors. They cross the FFI boundary untranslated.

pub use crate::sys::{
    GColor, GColor8, GColorFromHEX, GColorFromRGB, GColorFromRGBA, GPoint, GRect, GSize,
    GTextAlignment,
};

// The full 64-entry named palette, plus the FONT_KEY_* constants that
// `text_layer::system_font` takes. Both are demanded by the safe wrappers'
// own signatures, so app code that cannot reach them is forced into
// `ferrite::sys` -- which is the escape hatch, not the API.
pub use crate::sys::{
    GColorArmyGreen, GColorBabyBlueEyes, GColorBlack, GColorBlue, GColorBlueMoon, GColorBrass,
    GColorBrightGreen, GColorBrilliantRose, GColorBulgarianRose, GColorCadetBlue, GColorCeleste,
    GColorChromeYellow, GColorClear, GColorCobaltBlue, GColorCyan, GColorDarkCandyAppleRed,
    GColorDarkGray, GColorDarkGreen, GColorDukeBlue, GColorElectricBlue, GColorElectricUltramarine,
    GColorFashionMagenta, GColorFolly, GColorGreen, GColorIcterine, GColorImperialPurple,
    GColorInchworm, GColorIndigo, GColorIslamicGreen, GColorJaegerGreen, GColorJazzberryJam,
    GColorKellyGreen, GColorLavenderIndigo, GColorLiberty, GColorLightGray, GColorLimerick,
    GColorMagenta, GColorMalachite, GColorMayGreen, GColorMediumAquamarine,
    GColorMediumSpringGreen, GColorMelon, GColorMidnightGreen, GColorMintGreen, GColorOrange,
    GColorOxfordBlue, GColorPastelYellow, GColorPictonBlue, GColorPurple, GColorPurpureus,
    GColorRajah, GColorRed, GColorRichBrilliantLavender, GColorRoseVale, GColorScreaminGreen,
    GColorShockingPink, GColorSpringBud, GColorSunsetOrange, GColorTiffanyBlue,
    GColorVeryLightBlue, GColorVividCerulean, GColorVividViolet, GColorWhite, GColorWindsorTan,
    GColorYellow,
};

// The complete set of FONT_KEY_* constants. Includes internal-API fallback
// fonts for completeness; apps should use the public fonts instead (GOTHIC_*,
// BITHAM_*, LECO_*, ROBOTO_*, DROID_SERIF_*).
pub use crate::sys::{
    FONT_KEY_BITHAM_18_LIGHT_SUBSET, FONT_KEY_BITHAM_30_BLACK, FONT_KEY_BITHAM_34_LIGHT_SUBSET,
    FONT_KEY_BITHAM_34_MEDIUM_NUMBERS, FONT_KEY_BITHAM_42_BOLD, FONT_KEY_BITHAM_42_LIGHT,
    FONT_KEY_BITHAM_42_MEDIUM_NUMBERS, FONT_KEY_DROID_SERIF_28_BOLD, FONT_KEY_FONT_FALLBACK,
    FONT_KEY_FONT_FALLBACK_INTERNAL, FONT_KEY_GOTHIC_09, FONT_KEY_GOTHIC_14,
    FONT_KEY_GOTHIC_14_BOLD, FONT_KEY_GOTHIC_18, FONT_KEY_GOTHIC_18_BOLD, FONT_KEY_GOTHIC_24,
    FONT_KEY_GOTHIC_24_BOLD, FONT_KEY_GOTHIC_28, FONT_KEY_GOTHIC_28_BOLD,
    FONT_KEY_LECO_20_BOLD_NUMBERS, FONT_KEY_LECO_26_BOLD_NUMBERS_AM_PM,
    FONT_KEY_LECO_28_LIGHT_NUMBERS, FONT_KEY_LECO_32_BOLD_NUMBERS, FONT_KEY_LECO_36_BOLD_NUMBERS,
    FONT_KEY_LECO_38_BOLD_NUMBERS, FONT_KEY_LECO_42_NUMBERS, FONT_KEY_LECO_60_BOLD_NUMBERS_AM_PM,
    FONT_KEY_LECO_60_NUMBERS_AM_PM, FONT_KEY_ROBOTO_BOLD_SUBSET_49, FONT_KEY_ROBOTO_CONDENSED_21,
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

    /// Compile-time proof that the types the safe wrappers demand are
    /// reachable without `ferrite::sys`. Every name here appeared in a
    /// wrapper signature but was previously unreachable from app code.
    #[test]
    fn wrapper_facing_types_are_reachable() {
        let _: super::GColor8 = super::GColorYellow;
        let _: super::GColor8 = super::GColorRed;
        let _: super::GColor8 = super::GColorGreen;
        let _: super::GColor8 = super::GColorLightGray;
        let _: super::GTextAlignment = super::GTextAlignment::GTextAlignmentCenter;
    }
}
