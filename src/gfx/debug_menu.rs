use super::{camera::Camera, font::Font};
use cgmath::{Deg, Rad};
use wgpu_glyph::{Section, Text};

pub struct DebugMenu {
    pub enabled: bool,
    pub fps: usize,
}

impl DebugMenu {
    pub fn new() -> Self {
        Self {
            enabled: false,
            fps: 0,
        }
    }

    pub fn render(&self, bounds: (f32, f32), camera: &Camera, font: &mut Font) {
        if !self.enabled {
            return;
        }

        let mut offset = 0.0;

        let mut add_text = |txt: &str| {
            offset += 2.0;

            font.add(Section {
                screen_position: (2.0, offset),
                bounds,
                text: vec![Text::new(txt)
                    .with_color([1.0, 1.0, 1.0, 1.0])
                    .with_scale(20.0)],
                ..Section::default()
            });

            offset += 20.0;
        };

        let angle = |x| Deg::from(Rad(x)).0;

        let pos = camera.first_person.position;

        add_text(&format!(
            "{} {}",
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION")
        ));
        add_text(&format!("{} FPS", self.fps));
        add_text(&format!("({:.1}, {:.1}, {:.1})", pos[0], pos[1], pos[2]));
        add_text(&format!(
            "yaw: {:.1}°",
            (angle(camera.first_person.yaw) + 360.0) % 360.0
        ));
        add_text(&format!("pitch: {:.1}°", angle(camera.first_person.pitch)));
    }
}
