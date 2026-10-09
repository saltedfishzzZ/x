//! TS04：分片线性表面的面积—势位与容积—势位积分实验。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Triangle(pub [Vertex; 3]);

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapacitySample {
    pub potential: f64,
    pub area: f64,
    pub volume: f64,
}

fn interpolate(a: Vertex, b: Vertex, level: f64) -> Vertex {
    let fraction = (level - a.height) / (b.height - a.height);
    Vertex {
        x: a.x + fraction * (b.x - a.x),
        y: a.y + fraction * (b.y - a.y),
        height: level,
    }
}

fn clip_below(vertices: &[Vertex], level: f64) -> Vec<Vertex> {
    let mut output = Vec::new();
    for index in 0..vertices.len() {
        let current = vertices[index];
        let previous = vertices[(index + vertices.len() - 1) % vertices.len()];
        let current_inside = current.height <= level;
        let previous_inside = previous.height <= level;
        match (previous_inside, current_inside) {
            (true, true) => output.push(current),
            (true, false) => output.push(interpolate(previous, current, level)),
            (false, true) => {
                output.push(interpolate(previous, current, level));
                output.push(current);
            }
            (false, false) => {}
        }
    }
    output
}

fn signed_double_area(a: Vertex, b: Vertex, c: Vertex) -> f64 {
    (b.x - a.x).mul_add(c.y - a.y, -(b.y - a.y) * (c.x - a.x))
}

#[must_use]
pub fn triangle_capacity(triangle: Triangle, level: f64) -> CapacitySample {
    let polygon = clip_below(&triangle.0, level);
    if polygon.len() < 3 {
        return CapacitySample {
            potential: level,
            area: 0.0,
            volume: 0.0,
        };
    }
    let anchor = polygon[0];
    let mut area = 0.0;
    let mut volume = 0.0;
    for index in 1..polygon.len() - 1 {
        let b = polygon[index];
        let c = polygon[index + 1];
        let triangle_area = 0.5 * signed_double_area(anchor, b, c).abs();
        let average_depth = level - (anchor.height + b.height + c.height) / 3.0;
        area += triangle_area;
        volume += triangle_area * average_depth;
    }
    CapacitySample {
        potential: level,
        area,
        volume,
    }
}

#[must_use]
pub fn surface_capacity(triangles: &[Triangle], levels: &[f64]) -> Vec<CapacitySample> {
    levels
        .iter()
        .map(|&level| {
            triangles.iter().fold(
                CapacitySample {
                    potential: level,
                    area: 0.0,
                    volume: 0.0,
                },
                |mut total, triangle| {
                    let sample = triangle_capacity(*triangle, level);
                    total.area += sample.area;
                    total.volume += sample.volume;
                    total
                },
            )
        })
        .collect()
}

#[must_use]
pub fn is_non_negative_monotone(samples: &[CapacitySample], tolerance: f64) -> bool {
    samples
        .iter()
        .all(|sample| sample.area >= -tolerance && sample.volume >= -tolerance)
        && samples
            .windows(2)
            .all(|pair| pair[1].volume + tolerance >= pair[0].volume)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Triangle {
        Triangle([
            Vertex {
                x: 0.0,
                y: 0.0,
                height: 0.0,
            },
            Vertex {
                x: 1.0,
                y: 0.0,
                height: 1.0,
            },
            Vertex {
                x: 0.0,
                y: 1.0,
                height: 1.0,
            },
        ])
    }

    #[test]
    fn dry_below_minimum_and_full_above_maximum() {
        let dry = triangle_capacity(ramp(), -1.0);
        assert!(dry.area.abs() < f64::EPSILON);
        assert!(dry.volume.abs() < f64::EPSILON);
        let full = triangle_capacity(ramp(), 2.0);
        assert!((full.area - 0.5).abs() < 1.0e-15);
        assert!((full.volume - 2.0 / 3.0).abs() < 1.0e-15);
    }

    #[test]
    fn half_level_matches_analytic_subtriangle() {
        let sample = triangle_capacity(ramp(), 0.5);
        assert!((sample.area - 0.125).abs() < 1.0e-15);
        assert!((sample.volume - 1.0 / 48.0).abs() < 1.0e-15);
    }

    #[test]
    fn capacity_curve_is_monotone() {
        let levels: Vec<_> = (-2..=12).map(|i| f64::from(i) / 10.0).collect();
        let samples = surface_capacity(&[ramp()], &levels);
        assert!(is_non_negative_monotone(&samples, 1.0e-14));
    }

    #[test]
    fn injected_non_monotonicity_is_rejected() {
        let mut samples = surface_capacity(&[ramp()], &[0.0, 0.5, 1.0]);
        samples[2].volume = -1.0;
        assert!(!is_non_negative_monotone(&samples, 1.0e-14));
    }
}
