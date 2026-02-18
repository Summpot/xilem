// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use masonry_core::app::{RenderRootOptions, WindowSizePolicy};
use masonry_core::core::DefaultProperties;
use masonry_core::vello::{Error, wgpu};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::window::Window as WinitWindow;

use crate::vello_util::{RenderContext, RenderSurface};

/// Metrics captured from an externally owned winit window.
#[derive(Debug, Clone, Copy)]
pub struct ExistingWindowMetrics {
    /// Current window physical size.
    pub physical_size: PhysicalSize<u32>,
    /// Current window logical size.
    pub logical_size: LogicalSize<f64>,
    /// Current window scale factor.
    pub scale_factor: f64,
}

/// Capture size/scale metrics from an externally owned winit window.
pub fn existing_window_metrics(window: &WinitWindow) -> ExistingWindowMetrics {
    let physical_size = window.inner_size();
    let scale_factor = window.scale_factor();
    let logical_size = physical_size.to_logical(scale_factor);

    ExistingWindowMetrics {
        physical_size,
        logical_size,
        scale_factor,
    }
}

/// Build [`RenderRootOptions`] from an externally owned winit window.
pub fn render_root_options_from_existing_window(
    default_properties: Arc<DefaultProperties>,
    window: &WinitWindow,
    use_system_fonts: bool,
) -> RenderRootOptions {
    let metrics = existing_window_metrics(window);

    RenderRootOptions {
        default_properties,
        use_system_fonts,
        size_policy: WindowSizePolicy::User,
        size: PhysicalSize::new(
            metrics.physical_size.width.max(1),
            metrics.physical_size.height.max(1),
        ),
        scale_factor: metrics.scale_factor,
        test_font: None,
    }
}

/// A Vello surface context attached to an externally owned winit window.
///
/// This allows host frameworks (for example Bevy) to keep window/event-loop ownership,
/// while Masonry/Vello initializes rendering resources against that existing window.
pub struct ExternalWindowSurface {
    window: Arc<WinitWindow>,
    render_cx: RenderContext,
    surface: RenderSurface<'static>,
    scale_factor: f64,
}

impl ExternalWindowSurface {
    /// Create an attached Vello surface for an existing window.
    pub fn new(window: Arc<WinitWindow>, present_mode: wgpu::PresentMode) -> Result<Self, Error> {
        let mut render_cx = RenderContext::new();
        let metrics = existing_window_metrics(&window);
        let surface = pollster::block_on(render_cx.create_surface(
            window.clone(),
            metrics.physical_size.width.max(1),
            metrics.physical_size.height.max(1),
            present_mode,
        ))?;

        Ok(Self {
            window,
            render_cx,
            surface,
            scale_factor: metrics.scale_factor,
        })
    }

    /// Access the attached window.
    pub fn window(&self) -> &Arc<WinitWindow> {
        &self.window
    }

    /// Current physical size of the attached surface.
    pub fn physical_size(&self) -> PhysicalSize<u32> {
        PhysicalSize::new(self.surface.config.width, self.surface.config.height)
    }

    /// Current logical size of the attached surface.
    pub fn logical_size(&self) -> LogicalSize<f64> {
        self.physical_size().to_logical(self.scale_factor)
    }

    /// Current scale factor tracked for the attached window.
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Synchronize internal surface size and scale-factor from the attached window.
    pub fn sync_window_metrics(&mut self) -> ExistingWindowMetrics {
        let metrics = existing_window_metrics(&self.window);
        self.scale_factor = metrics.scale_factor;

        if self.surface.config.width != metrics.physical_size.width
            || self.surface.config.height != metrics.physical_size.height
        {
            self.render_cx.resize_surface(
                &mut self.surface,
                metrics.physical_size.width.max(1),
                metrics.physical_size.height.max(1),
            );
        }

        metrics
    }
}
