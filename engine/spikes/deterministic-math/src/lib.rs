//! TS02：命名随机键、固定归约与最小实球谐求值实验。

use sha2::{Digest, Sha256};
use sphere_core::Vec3;

#[must_use]
pub fn named_u64(world_seed: u64, stream: &str, coordinates: &[u64]) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(world_seed.to_le_bytes());
    hasher.update((stream.len() as u64).to_le_bytes());
    hasher.update(stream.as_bytes());
    for coordinate in coordinates {
        hasher.update(coordinate.to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(prefix)
}

#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn named_unit_f64(world_seed: u64, stream: &str, coordinates: &[u64]) -> f64 {
    // `bits` is deliberately limited to the exact 53-bit integer range of f64.
    let bits = named_u64(world_seed, stream, coordinates) >> 11;
    bits as f64 * (1.0 / ((1_u64 << 53) as f64))
}

/// Sum values after canonical key ordering. Kahan compensation makes the result
/// independent from caller iteration and task completion order.
#[must_use]
pub fn stable_sum(values: &[(u64, f64)]) -> f64 {
    let mut ordered = values.to_vec();
    ordered.sort_by_key(|(key, _)| *key);
    let mut sum = 0.0;
    let mut correction = 0.0;
    for (_, value) in ordered {
        let adjusted = value - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
    }
    sum
}

fn associated_legendre(l: u32, m: u32, x: f64) -> f64 {
    assert!(m <= l);
    let mut diagonal = 1.0;
    if m > 0 {
        let root = (1.0 - x * x).max(0.0).sqrt();
        let mut factor = 1.0;
        for _ in 1..=m {
            diagonal *= -factor * root;
            factor += 2.0;
        }
    }
    if l == m {
        return diagonal;
    }
    let next_degree = x * f64::from(2 * m + 1) * diagonal;
    if l == m + 1 {
        return next_degree;
    }
    let mut previous = diagonal;
    let mut current = next_degree;
    for degree in (m + 2)..=l {
        let next = (f64::from(2 * degree - 1) * x * current - f64::from(degree + m - 1) * previous)
            / f64::from(degree - m);
        previous = current;
        current = next;
    }
    current
}

/// Unnormalised real spherical-harmonic basis used only by the TS02 spike.
///
/// # Panics
/// Panics when `|m| > l`; fixture construction treats this as a programmer
/// error rather than an unresolved scientific state.
#[must_use]
pub fn real_spherical_basis(l: u32, m: i32, position: Vec3) -> f64 {
    assert!(m.unsigned_abs() <= l);
    let p = position.normalized();
    let longitude = p.y.atan2(p.x);
    let legendre = associated_legendre(l, m.unsigned_abs(), p.z);
    match m.cmp(&0) {
        std::cmp::Ordering::Greater => legendre * (f64::from(m) * longitude).cos(),
        std::cmp::Ordering::Less => legendre * (f64::from(-m) * longitude).sin(),
        std::cmp::Ordering::Equal => legendre,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_random_is_order_independent_and_stream_isolated() {
        let a = named_u64(7, "relief.phase", &[3, 4]);
        assert_eq!(a, named_u64(7, "relief.phase", &[3, 4]));
        assert_ne!(a, named_u64(7, "relief.amplitude", &[3, 4]));
        assert_ne!(a, named_u64(7, "relief.phase", &[4, 3]));
    }

    #[test]
    fn stable_sum_ignores_input_order() {
        let forward = vec![(1, 1.0e16), (2, 1.0), (3, -1.0e16), (4, 2.0)];
        let mut reverse = forward.clone();
        reverse.reverse();
        assert_eq!(
            stable_sum(&forward).to_bits(),
            stable_sum(&reverse).to_bits()
        );
    }

    #[test]
    fn named_values_reduce_identically_with_one_two_or_four_workers() {
        fn run(worker_count: usize) -> u64 {
            let mut handles = Vec::new();
            for worker in 0..worker_count {
                handles.push(std::thread::spawn(move || {
                    (worker..4_096)
                        .step_by(worker_count)
                        .map(|index| {
                            let key = index as u64;
                            (key, named_unit_f64(91, "ts02.thread", &[key]))
                        })
                        .collect::<Vec<_>>()
                }));
            }
            let values: Vec<_> = handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect();
            stable_sum(&values).to_bits()
        }

        assert_eq!(run(1), run(2));
        assert_eq!(run(1), run(4));
    }

    #[test]
    fn spherical_basis_has_expected_low_order_values() {
        let north = Vec3::new(0.0, 0.0, 1.0);
        assert!((real_spherical_basis(0, 0, north) - 1.0).abs() < f64::EPSILON);
        assert!((real_spherical_basis(1, 0, north) - 1.0).abs() < f64::EPSILON);
        assert!(real_spherical_basis(1, 1, north).abs() < f64::EPSILON);
    }
}
