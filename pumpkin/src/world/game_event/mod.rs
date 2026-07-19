use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

/// `distanceBetweenInBlocks` — vanilla `Mth.floor(sqrt(distSqr))`.
#[must_use]
pub fn distance_between_blocks(a: &BlockPos, b: &BlockPos) -> f32 {
    let dx = a.0.x as f32 - b.0.x as f32;
    let dy = a.0.y as f32 - b.0.y as f32;
    let dz = a.0.z as f32 - b.0.z as f32;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[must_use]
pub fn dist_sq(a: &Vector3<f64>, b: &Vector3<f64>) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    dx * dx + dy * dy + dz * dz
}

pub mod vibration;
