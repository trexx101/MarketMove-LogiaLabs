//! Trailing stop with hysteresis
//!
//! Re-arm requires recovery band (0.5 × ATR above stop) to prevent
//! whipsaw churn through spreads.

use crate::options::exit_arbiter::{ExitSignal, ExitSource};
use chrono::Utc;

#[cfg(test)]
mod tests;

/// Trailing stop state for a single position
#[derive(Debug, Clone)]
pub struct TrailingStop {
    /// Current stop level (price)
    pub stop_price: f64,
    /// Highest observed price since entry (for calls) or lowest (for puts)
    pub high_water_mark: f64,
    /// ATR at entry time (used for hysteresis band)
    pub atr: f64,
    /// Whether the stop is armed (active) or disarmed (recovering)
    pub armed: bool,
    /// Recovery threshold: stop must recover by 0.5 × ATR to re-arm
    pub recovery_threshold: f64,
}

impl TrailingStop {
    /// Create a new trailing stop
    ///
    /// # Arguments
    /// * `initial_price` - Entry price of the underlying
    /// * `atr` - ATR at entry time (for hysteresis calculation)
    /// * `is_call` - true for call options, false for puts
    pub fn new(initial_price: f64, atr: f64, is_call: bool) -> Self {
        let stop_price = if is_call {
            initial_price * 0.95 // 5% trailing stop for calls
        } else {
            initial_price * 1.05 // 5% trailing stop for puts
        };

        Self {
            stop_price,
            high_water_mark: initial_price,
            atr,
            armed: true,
            recovery_threshold: atr * 0.5,
        }
    }

    /// Update the trailing stop with a new price observation
    ///
    /// Returns Some(ExitSignal) if the stop is triggered, None otherwise.
    pub fn update(&mut self, current_price: f64, is_call: bool) -> Option<ExitSignal> {
        if !self.armed {
            // Check if we've recovered enough to re-arm
            let recovery_needed = if is_call {
                current_price >= self.stop_price + self.recovery_threshold
            } else {
                current_price <= self.stop_price - self.recovery_threshold
            };

            if recovery_needed {
                self.armed = true;
                // Reset high water mark to current price
                self.high_water_mark = current_price;
            } else {
                return None;
            }
        }

        // Update high water mark
        if is_call {
            if current_price > self.high_water_mark {
                self.high_water_mark = current_price;
                // Trail the stop up
                self.stop_price = self.high_water_mark * 0.95;
            }
        } else {
            if current_price < self.high_water_mark {
                self.high_water_mark = current_price;
                // Trail the stop down
                self.stop_price = self.high_water_mark * 1.05;
            }
        }

        // Check if stop is triggered
        let triggered = if is_call {
            current_price <= self.stop_price
        } else {
            current_price >= self.stop_price
        };

        if triggered {
            self.armed = false; // Disarm until recovery
            Some(ExitSignal {
                source: ExitSource::TrailingStop,
                priority: 4,
                reason: format!(
                    "Trailing stop triggered at {:.2} (stop: {:.2}, hwm: {:.2})",
                    current_price, self.stop_price, self.high_water_mark
                ),
                timestamp: Utc::now(),
            })
        } else {
            None
        }
    }
}
