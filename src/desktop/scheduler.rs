//! Deadline-based observation cadence skips missed ticks instead of accumulating work.
use std::time::Duration;
use tokio::time::{Interval, MissedTickBehavior};

pub(crate) struct ObservationSchedule(Interval);

impl ObservationSchedule {
    pub fn new(period: Duration) -> Self {
        let mut interval = tokio::time::interval(period);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        Self(interval)
    }

    pub async fn wait(&mut self) {
        self.0.tick().await;
    }
}
