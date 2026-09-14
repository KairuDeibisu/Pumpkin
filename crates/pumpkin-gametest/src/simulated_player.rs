use pumpkin_util::math::vector3::Vector3;
use std::sync::Mutex;

/// Shared movement state for a server-side simulated player.
///
/// The controller is intentionally independent of Pumpkin's network player type so it can be
/// reused by native Rust tests and plugin-language adapters.
pub struct SimulatedPlayerController {
    target: Mutex<Option<Vector3<f64>>>,
    speed_per_tick: f64,
}

impl Default for SimulatedPlayerController {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulatedPlayerController {
    /// Default movement speed in blocks per server tick.
    pub const DEFAULT_SPEED_PER_TICK: f64 = 0.1;

    /// Creates a controller with the default movement speed.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            target: Mutex::new(None),
            speed_per_tick: Self::DEFAULT_SPEED_PER_TICK,
        }
    }

    /// Sets the world position the simulated player should move toward.
    pub fn move_to_location(&self, target: Vector3<f64>) {
        *self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(target);
    }

    /// Returns the collision-aware movement delta to attempt this tick.
    #[must_use]
    pub fn next_step(&self, current: Vector3<f64>) -> Option<Vector3<f64>> {
        let target = *self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let target = target?;

        let dx = target.x - current.x;
        let dy = target.y - current.y;
        let dz = target.z - current.z;
        let distance_squared = dx * dx + dy * dy + dz * dz;
        if distance_squared <= f64::EPSILON {
            self.clear_target();
            return None;
        }

        let distance = distance_squared.sqrt();
        let step = self.speed_per_tick.min(distance);
        Some(Vector3::new(
            dx / distance * step,
            dy / distance * step,
            dz / distance * step,
        ))
    }

    /// Clears the target once the actor has actually reached it after collision handling.
    pub fn finish_step(&self, current: Vector3<f64>) {
        let mut target = self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(destination) = *target else {
            return;
        };

        let dx = destination.x - current.x;
        let dy = destination.y - current.y;
        let dz = destination.z - current.z;
        if dx * dx + dy * dy + dz * dz <= 1.0e-8 {
            *target = None;
        }
    }

    fn clear_target(&self) {
        *self
            .target
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}
