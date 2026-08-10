//! Safe drawing context, handed to canvas update closures.
//!
//! Wraps the SDK `GContext`, which is only valid for the duration of an
//! update proc — the lifetime parameter enforces that borrow structurally.

use core::marker::PhantomData;

use crate::sys;

pub struct Graphics<'a> {
    ctx: *mut sys::GContext,
    bounds: sys::GRect,
    _lifetime: PhantomData<&'a mut ()>,
}

impl<'a> Graphics<'a> {
    /// Internal: constructed by trampolines only.
    pub(crate) fn new(ctx: *mut sys::GContext, bounds: sys::GRect) -> Graphics<'a> {
        Graphics {
            ctx,
            bounds,
            _lifetime: PhantomData,
        }
    }

    /// Bounds of the layer being drawn.
    pub fn bounds(&self) -> sys::GRect {
        self.bounds
    }

    pub fn set_stroke_color(&mut self, color: sys::GColor8) {
        unsafe { sys::graphics_context_set_stroke_color(self.ctx, color) };
    }

    pub fn set_fill_color(&mut self, color: sys::GColor8) {
        unsafe { sys::graphics_context_set_fill_color(self.ctx, color) };
    }

    pub fn set_stroke_width(&mut self, width: u8) {
        unsafe { sys::graphics_context_set_stroke_width(self.ctx, width) };
    }

    pub fn set_antialiased(&mut self, enabled: bool) {
        unsafe { sys::graphics_context_set_antialiased(self.ctx, enabled) };
    }

    pub fn draw_pixel(&mut self, point: sys::GPoint) {
        unsafe { sys::graphics_draw_pixel(self.ctx, point) };
    }

    pub fn draw_line(&mut self, from: sys::GPoint, to: sys::GPoint) {
        unsafe { sys::graphics_draw_line(self.ctx, from, to) };
    }

    pub fn draw_rect(&mut self, rect: sys::GRect) {
        unsafe { sys::graphics_draw_rect(self.ctx, rect) };
    }

    /// Fills a rectangle; `corner_radius` 0 and `GCornerNone` for square.
    pub fn fill_rect(&mut self, rect: sys::GRect, corner_radius: u16, mask: sys::GCornerMask) {
        unsafe { sys::graphics_fill_rect(self.ctx, rect, corner_radius, mask) };
    }

    pub fn draw_circle(&mut self, center: sys::GPoint, radius: u16) {
        unsafe { sys::graphics_draw_circle(self.ctx, center, radius) };
    }

    pub fn fill_circle(&mut self, center: sys::GPoint, radius: u16) {
        unsafe { sys::graphics_fill_circle(self.ctx, center, radius) };
    }
}
