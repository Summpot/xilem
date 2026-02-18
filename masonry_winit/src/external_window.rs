// Copyright 2026 the Xilem Authors
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use masonry_core::app::{RenderRootOptions, WindowSizePolicy};
use masonry_core::core::DefaultProperties;
use masonry_core::kurbo::Affine;
use masonry_core::peniko::Color;
use masonry_core::vello::{
    wgpu, AaConfig, AaSupport, Error, RenderParams, Renderer, RendererOptions, Scene,
};
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

    /// Render a Masonry/Vello scene and present it to the attached window surface.
    pub fn render_scene(
        &mut self,
        renderer: &mut Option<Renderer>,
        scene: Scene,
        logical_width: u32,
        logical_height: u32,
        base_color: Color,
    ) {
        let transformed_scene = if self.scale_factor == 1.0 {
            None
        } else {
            let mut scaled = Scene::new();
            scaled.append(&scene, Some(Affine::scale(self.scale_factor)));
            Some(scaled)
        };
        let scene_ref = transformed_scene.as_ref().unwrap_or(&scene);

        let dev_id = self.surface.dev_id;
        let device = &self.render_cx.devices[dev_id].device;
        let queue = &self.render_cx.devices[dev_id].queue;

        let renderer = renderer.get_or_insert_with(|| {
            Renderer::new(
                device,
                RendererOptions {
                    antialiasing_support: AaSupport::area_only(),
                    ..Default::default()
                },
            )
            .expect("failed to create Vello renderer")
        });

        let render_params = RenderParams {
            base_color,
            width: logical_width.max(1),
            height: logical_height.max(1),
            antialiasing_method: AaConfig::Area,
        };

        let surface_texture = match self.surface.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Outdated) => {
                let size = self.window.inner_size();
                self.render_cx.resize_surface(
                    &mut self.surface,
                    size.width.max(1),
                    size.height.max(1),
                );

                match self.surface.surface.get_current_texture() {
                    Ok(texture) => texture,
                    Err(err) => {
                        tracing::error!(
                            "Couldn't get swap chain texture after configuring. Cause: '{err}'"
                        );
                        return;
                    }
                }
            }
            Err(err) => {
                tracing::error!("Couldn't get swap chain texture, operation unrecoverable: {err}");
                return;
            }
        };

        if let Err(err) = renderer.render_to_texture(
            device,
            queue,
            scene_ref,
            &self.surface.target_view,
            &render_params,
        ) {
            tracing::error!("failed to render scene to texture: {err}");
            return;
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("External Window Surface Blit"),
        });
        self.surface.blitter.copy(
            device,
            &mut encoder,
            &self.surface.target_view,
            &surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default()),
        );
        queue.submit([encoder.finish()]);
        self.window.pre_present_notify();
        surface_texture.present();

        if let Err(err) = device.poll(wgpu::PollType::wait_indefinitely()) {
            tracing::error!("error while waiting for GPU completion: {err}");
        }
    }
}
