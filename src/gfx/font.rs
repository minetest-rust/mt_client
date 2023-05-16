use super::gpu::{Frame, Gpu};

pub struct Font {
    glyph_brush: wgpu_glyph::GlyphBrush<()>,
    staging_belt: wgpu::util::StagingBelt,
}

impl Font {
    pub fn new(gpu: &Gpu) -> Self {
        Self {
            glyph_brush: wgpu_glyph::GlyphBrushBuilder::using_font(
                wgpu_glyph::ab_glyph::FontArc::try_from_slice(include_bytes!(
                    "../../assets/font/regular.otf"
                ))
                .unwrap(),
            )
            .build(&gpu.device, gpu.config.format),
            staging_belt: wgpu::util::StagingBelt::new(1024),
        }
    }

    pub fn add(&mut self, section: wgpu_glyph::Section) {
        self.glyph_brush.queue(section);
    }

    pub fn submit(&mut self, frame: &mut Frame) {
        self.glyph_brush
            .draw_queued(
                &frame.gpu.device,
                &mut self.staging_belt,
                &mut frame.encoder,
                &frame.view,
                frame.gpu.config.width,
                frame.gpu.config.height,
            )
            .unwrap();

        self.staging_belt.finish();
    }

    pub fn cleanup(&mut self) {
        self.staging_belt.recall();
    }
}
