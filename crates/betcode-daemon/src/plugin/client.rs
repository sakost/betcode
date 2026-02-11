use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginStatus {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct PluginHealth {
    pub consecutive_failures: u32,
    pub degraded_threshold: u32,
    pub unavailable_threshold: u32,
}

impl PluginHealth {
    pub const fn new(degraded_threshold: u32, unavailable_threshold: u32) -> Self {
        Self {
            consecutive_failures: 0,
            degraded_threshold,
            unavailable_threshold,
        }
    }

    pub const fn status(&self) -> PluginStatus {
        if self.consecutive_failures >= self.unavailable_threshold {
            PluginStatus::Unavailable
        } else if self.consecutive_failures >= self.degraded_threshold {
            PluginStatus::Degraded
        } else {
            PluginStatus::Healthy
        }
    }

    pub const fn record_failure(&mut self) {
        self.consecutive_failures += 1;
    }

    pub const fn record_success(&mut self) {
        self.consecutive_failures = 0;
    }

    pub const fn reset(&mut self) {
        self.consecutive_failures = 0;
    }
}

pub struct PluginClient {
    pub name: String,
    pub socket_path: String,
    pub health: PluginHealth,
    pub timeout: Duration,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_health_state_machine() {
        let mut health = PluginHealth::new(3, 10);
        assert_eq!(health.status(), PluginStatus::Healthy);
        health.record_failure();
        health.record_failure();
        health.record_failure();
        assert_eq!(health.status(), PluginStatus::Degraded);
        for _ in 0..7 {
            health.record_failure();
        }
        assert_eq!(health.status(), PluginStatus::Unavailable);
        health.reset();
        assert_eq!(health.status(), PluginStatus::Healthy);
    }

    #[test]
    fn test_plugin_health_success_resets_failures() {
        let mut health = PluginHealth::new(3, 10);
        health.record_failure();
        health.record_failure();
        health.record_success();
        assert_eq!(health.status(), PluginStatus::Healthy);
    }
}
