use image::{Rgba, RgbaImage};

pub const VIEW_BOX: &str = "0 0 256 256";
pub const BOLT_POINTS: [(f32, f32); 4] =
    [(171.0, 49.0), (102.0, 115.0), (154.0, 115.0), (87.0, 207.0)];
pub const NODE_INDICES: [usize; 3] = [0, 2, 3];
pub const STROKE_WIDTH: f32 = 26.0;
pub const STROKE_RADIUS: f32 = STROKE_WIDTH / 2.0;
pub const NODE_RADIUS: f32 = 17.0;
pub const TILE_BG: [u8; 4] = [36, 39, 45, 255];

pub fn svg_points() -> String {
    BOLT_POINTS
        .iter()
        .map(|(x, y)| format!("{x:.0},{y:.0}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn distance_sq_to_segment(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let ab = (b.0 - a.0, b.1 - a.1);
    let ap = (px - a.0, py - a.1);
    let length_sq = ab.0 * ab.0 + ab.1 * ab.1;
    let t = ((ap.0 * ab.0 + ap.1 * ab.1) / length_sq).clamp(0.0, 1.0);
    let dx = px - (a.0 + t * ab.0);
    let dy = py - (a.1 + t * ab.1);
    dx * dx + dy * dy
}

pub fn mark_contains(x: f32, y: f32) -> bool {
    BOLT_POINTS
        .windows(2)
        .any(|s| distance_sq_to_segment(x, y, s[0], s[1]) <= STROKE_RADIUS.powi(2))
        || NODE_INDICES.iter().any(|&i| {
            let (nx, ny) = BOLT_POINTS[i];
            (x - nx).powi(2) + (y - ny).powi(2) <= NODE_RADIUS.powi(2)
        })
}

fn rounded_tile_contains(x: f32, y: f32) -> bool {
    const MIN: f32 = 8.0;
    const MAX: f32 = 248.0;
    const RADIUS: f32 = 42.0;
    let nearest_x = x.clamp(MIN + RADIUS, MAX - RADIUS);
    let nearest_y = y.clamp(MIN + RADIUS, MAX - RADIUS);
    x >= MIN
        && x <= MAX
        && y >= MIN
        && y <= MAX
        && (x - nearest_x).powi(2) + (y - nearest_y).powi(2) <= RADIUS.powi(2)
}

pub fn render_tile(size: u32) -> RgbaImage {
    const SAMPLES: u32 = 4;
    let mut image = RgbaImage::new(size, size);
    for py in 0..size {
        for px in 0..size {
            let mut rgb = [0u32; 3];
            let mut covered = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32) * 256.0 / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32) * 256.0 / size as f32;
                    let sample = if mark_contains(x, y) {
                        Some([255, 255, 255])
                    } else if rounded_tile_contains(x, y) {
                        Some([TILE_BG[0], TILE_BG[1], TILE_BG[2]])
                    } else {
                        None
                    };
                    if let Some(sample) = sample {
                        covered += 1;
                        for channel in 0..3 {
                            rgb[channel] += u32::from(sample[channel]);
                        }
                    }
                }
            }
            let count = SAMPLES * SAMPLES;
            let pixel = if covered == 0 {
                [0, 0, 0, 0]
            } else {
                [
                    (rgb[0] / covered) as u8,
                    (rgb[1] / covered) as u8,
                    (rgb[2] / covered) as u8,
                    (covered * 255 / count) as u8,
                ]
            };
            image.put_pixel(px, py, Rgba(pixel));
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bolt_is_open_and_nodes_are_on_the_path() {
        assert_eq!(BOLT_POINTS.len(), 4);
        assert_ne!(BOLT_POINTS.first(), BOLT_POINTS.last());
        assert_eq!(NODE_INDICES, [0, 2, 3]);
        for index in NODE_INDICES {
            assert!(index < BOLT_POINTS.len());
        }
    }

    #[test]
    fn every_node_has_at_least_24_units_of_tile_padding() {
        const TILE_MIN: f32 = 8.0;
        const TILE_MAX: f32 = 248.0;
        const MIN_PADDING: f32 = 24.0;

        for index in NODE_INDICES {
            let (x, y) = BOLT_POINTS[index];
            assert!(x - NODE_RADIUS - TILE_MIN >= MIN_PADDING);
            assert!(y - NODE_RADIUS - TILE_MIN >= MIN_PADDING);
            assert!(TILE_MAX - (x + NODE_RADIUS) >= MIN_PADDING);
            assert!(TILE_MAX - (y + NODE_RADIUS) >= MIN_PADDING);
        }
    }

    #[test]
    fn tile_is_square_rgba_with_transparent_corners() {
        let tile = render_tile(64);
        assert_eq!(tile.dimensions(), (64, 64));
        assert_eq!(tile.get_pixel(0, 0).0[3], 0);
        assert_eq!(tile.get_pixel(32, 32).0[3], 255);
    }
}
