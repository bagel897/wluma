use crate::als::Als;
use crate::brightness::Brightness;
use crate::controller::data::{Data, Entry};
use crate::controller::kalman::Kalman;
use itertools::Itertools;
use std::cmp::{max, min};
use std::error::Error;
use std::thread;
use std::time::Duration;

const TRANSITION_SPEED: u64 = 200;
const PENDING_COOLDOWN_RESET: u8 = 15;

pub struct Controller {
    brightness: Box<dyn Brightness>,
    als: Box<dyn Als>,
    kalman: Kalman,
    last_brightness: u64,
    pending_cooldown: u8,
    pending: Option<Entry>,
    data: Data,
    stateful: bool,
}

impl Controller {
    pub fn new(brightness: Box<dyn Brightness>, als: Box<dyn Als>, stateful: bool) -> Self {
        let data = if stateful {
            Data::load().unwrap_or_default()
        } else {
            Data::default()
        };

        Self {
            brightness,
            als,
            kalman: Kalman::new(1., 20., 10.),
            last_brightness: 0,
            pending_cooldown: 0,
            pending: None,
            data,
            stateful,
        }
    }

    pub fn adjust(&mut self, luma: Option<u8>) -> Result<(), Box<dyn Error>> {
        let lux = self.als.get()?;
        let lux = self.kalman.process(lux as f64).round() as u64; // TODO make Kalman::<u64>
        let brightness = self.brightness.get().unwrap();
        if !self.kalman.initialized() {
            self.last_brightness = brightness;
            return Ok(());
        }

        self.process(lux, luma, brightness);

        Ok(())
    }

    fn process(&mut self, lux: u64, luma: Option<u8>, brightness: u64) {
        let user_changed_brightness = self.last_brightness != brightness;
        let no_data_points = self.data.entries.is_empty() && self.pending.is_none();

        self.last_brightness = brightness;

        if user_changed_brightness || no_data_points {
            if self.pending.is_none() {
                // First time we notice user adjusting brightness, freeze lux and luma...
                self.pending = Some(Entry::new(lux, luma, brightness));
            } else {
                // ... but as user keeps changing brightness,
                // allow some time for them to reach the desired brightness level for a given lux and luma
                self.pending.as_mut().unwrap().brightness = brightness;
            }
            // Every time user changed brightness, reset the cooldown period
            self.pending_cooldown = PENDING_COOLDOWN_RESET;
        } else if self.pending_cooldown > 0 {
            self.pending_cooldown -= 1;
        } else if self.pending.is_some() {
            self.learn();
        } else {
            let desired_brightness = self.predict(lux, luma);
            self.change_brightness(brightness, desired_brightness);
            self.last_brightness = desired_brightness;
        }
    }

    fn learn(&mut self) {
        let pending = self.pending.take().expect("No pending entry to learn");

        self.data.entries.retain(|entry| {
            let darker_env_darker_screen = entry.lux < pending.lux && entry.luma < pending.luma;

            let darker_env_same_screen = entry.lux < pending.lux
                && entry.luma == pending.luma
                && entry.brightness <= pending.brightness;

            let darker_env_brighter_screen = entry.lux < pending.lux
                && entry.luma > pending.luma
                && entry.brightness <= pending.brightness;

            let same_env_darker_screen = entry.lux == pending.lux
                && entry.luma < pending.luma
                && entry.brightness >= pending.brightness;

            let same_env_brighter_screen = entry.lux == pending.lux
                && entry.luma > pending.luma
                && entry.brightness <= pending.brightness;

            let brighter_env_darker_screen = entry.lux > pending.lux
                && entry.luma < pending.luma
                && entry.brightness >= pending.brightness;

            let brighter_env_same_screen = entry.lux > pending.lux
                && entry.luma == pending.luma
                && entry.brightness >= pending.brightness;

            let brighter_env_brighter_screen = entry.lux > pending.lux && entry.luma > pending.luma;

            darker_env_darker_screen
                || darker_env_same_screen
                || darker_env_brighter_screen
                || same_env_darker_screen
                || same_env_brighter_screen
                || brighter_env_darker_screen
                || brighter_env_same_screen
                || brighter_env_brighter_screen
        });

        self.data.entries.push(pending);

        if self.stateful {
            self.data.save().expect("Unable to save data");
        }
    }

    fn predict(&mut self, lux: u64, luma: Option<u8>) -> u64 {
        if self.data.entries.len() == 0 {
            return 0;
        }

        let points = self
            .data
            .entries
            .iter()
            .map(|entry| {
                let p1 = lux as f64 - entry.lux as f64;
                let p2 = luma.unwrap_or(0) as f64 - entry.luma.unwrap_or(0) as f64;
                let distance = (p1.powf(2.0) + p2.powf(2.0)).sqrt();
                (entry.brightness as f64, distance)
            })
            .collect_vec();

        let points = points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let other_distances: f64 = points[0..i]
                    .iter()
                    .chain(&points[i + 1..])
                    .map(|p| p.1)
                    .product();
                (p.0, p.1, other_distances)
            })
            .collect_vec();

        let distance_denominator: f64 = points
            .iter()
            .map(|p| p.1)
            .combinations(points.len() - 1)
            .map(|c| c.iter().product::<f64>())
            .sum();

        let prediction: f64 = points
            .iter()
            .map(|p| p.0 * p.2 / distance_denominator)
            .sum();

        prediction as u64
    }

    fn change_brightness(&self, mut last_value: u64, value: u64) {
        if last_value == value {
            return;
        }

        let diff = max(value, last_value) - min(value, last_value);
        let dir = if last_value < value { 1 } else { -1 };
        let (step, sleep) = if diff >= TRANSITION_SPEED {
            (diff / TRANSITION_SPEED, 1)
        } else {
            (1, TRANSITION_SPEED / diff)
        };

        while dir > 0 && last_value < value || dir < 0 && last_value > value {
            let new_value = ((last_value as i64) + (step as i64) * dir) as u64;
            self.brightness.set(new_value).unwrap();
            last_value = new_value;
            thread::sleep(Duration::from_millis(sleep));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::als::MockAls;
    use crate::brightness::MockBrightness;
    use itertools::iproduct;
    use std::collections::HashSet;

    fn setup_controller() -> Controller {
        Controller::new(
            Box::new(MockBrightness::new()),
            Box::new(MockAls::new()),
            false,
        )
    }

    #[test]
    fn test_process_first_user_change() {
        let mut controller = setup_controller();

        // User changes brightness to value 33 for a given lux and luma
        controller.process(12345, Some(66), 33);

        assert_eq!(33, controller.last_brightness);
        assert_eq!(Some(Entry::new(12345, Some(66), 33)), controller.pending);
        assert_eq!(PENDING_COOLDOWN_RESET, controller.pending_cooldown);
    }

    #[test]
    fn test_process_several_continuous_user_changes() {
        let mut controller = setup_controller();

        // User initiates brightness change for a given lux and luma to value 33...
        controller.process(12345, Some(66), 33);
        // then quickly continues increasing it to 34 (while lux and luma might already be different)...
        controller.process(23456, Some(36), 34);
        // and once again increases to 35 (which is the indended brightness value they wish to learn for the initial lux and luma)
        controller.process(100, Some(16), 35);

        assert_eq!(35, controller.last_brightness);
        assert_eq!(Some(Entry::new(12345, Some(66), 35)), controller.pending);
        assert_eq!(PENDING_COOLDOWN_RESET, controller.pending_cooldown);
    }

    #[test]
    fn test_process_learns_user_change_after_cooldown() {
        let mut controller = setup_controller();

        // User changes brightness to a desired value
        controller.process(12345, Some(66), 33);
        controller.process(23456, Some(36), 34);
        controller.process(100, Some(16), 35);

        for i in 1..=PENDING_COOLDOWN_RESET {
            // User doesn't change brightness anymore, so even if lux or luma change, we are in cooldown period
            controller.process(100 + i as u64, Some(i), 35);
            assert_eq!(PENDING_COOLDOWN_RESET - i, controller.pending_cooldown);
            assert_eq!(Some(Entry::new(12345, Some(66), 35)), controller.pending);
        }

        // One final process will trigger the learning
        controller.process(200, Some(17), 35);

        assert_eq!(None, controller.pending);
        assert_eq!(0, controller.pending_cooldown);
        assert_eq!(
            vec![Entry::new(12345, Some(66), 35)],
            controller.data.entries
        );
    }

    // If user configured brightess value in certain conditions (amount of light around, screen contents),
    // how changes in environment or screen contents can affect the desired brightness level:
    //
    // |                 | darker env      | same env         | brighter env     |
    // | darker screen   | any             | same or brighter | same or brighter |
    // | same screen     | same or dimmer  | only same        | same or brighter |
    // | brighter screen | same or dimmer  | same or dimmer   | any              |

    #[test]
    fn test_learn_data_cleanup() {
        let mut controller = setup_controller();

        let pending = Entry::new(10, Some(20), 30);

        let all_combinations: HashSet<_> = iproduct!(-1i32..=1, -1i32..=1, -1i32..=1)
            .map(|(i, j, k)| Entry::new((10 + i) as u64, Some((20 + j) as u8), (30 + k) as u64))
            .collect();

        let to_be_deleted: HashSet<_> = vec![
            // darker env same screen
            Entry::new(9, Some(20), 31),
            // darker env brighter screen
            Entry::new(9, Some(21), 31),
            // same env darker screen
            Entry::new(10, Some(19), 29),
            // same env same screen
            Entry::new(10, Some(20), 29),
            Entry::new(10, Some(20), 31),
            // same env brighter screen
            Entry::new(10, Some(21), 31),
            // brighter env darker screen
            Entry::new(11, Some(19), 29),
            // brighter env same screen
            Entry::new(11, Some(20), 29),
        ]
        .into_iter()
        .collect();

        controller.data.entries = all_combinations.iter().cloned().collect_vec();
        controller.pending = Some(pending);

        controller.learn();

        let to_remain: HashSet<_> = all_combinations.difference(&to_be_deleted).collect();
        let remained = controller.data.entries.iter().collect();

        assert_eq!(
            Vec::<&&Entry>::new(),
            to_remain.difference(&remained).collect_vec(),
            "unexpected entries were removed"
        );

        assert_eq!(
            Vec::<&&Entry>::new(),
            remained.difference(&to_remain).collect_vec(),
            "some entries were not removed"
        );

        assert_eq!(
            to_remain.len(),
            controller.data.entries.len(),
            "duplicate entries remained"
        );
    }

    #[test]
    fn test_predict_no_data_points() {
        let mut controller = setup_controller();
        controller.data.entries = vec![];
        // predict() should not be called with no data, but just in case confirm we don't panic
        assert_eq!(0, controller.predict(10, Some(20)));
    }

    #[test]
    fn test_predict_one_data_point() {
        let mut controller = setup_controller();
        controller.data.entries = vec![Entry::new(5, Some(10), 15)];
        assert_eq!(15, controller.predict(10, Some(20)));
    }

    #[test]
    fn test_predict_known_conditions() {
        let mut controller = setup_controller();
        controller.data.entries = vec![Entry::new(5, Some(10), 15), Entry::new(10, Some(20), 30)];
        assert_eq!(30, controller.predict(10, Some(20)));
    }

    #[test]
    fn test_predict_approximate() {
        let mut controller = setup_controller();
        controller.data.entries = vec![
            Entry::new(5, Some(10), 15),
            Entry::new(10, Some(20), 30),
            Entry::new(100, Some(100), 100),
        ];

        // Approximated using weighted distance to all known points:
        // dist1 = sqrt((x1 - x2)^2 + (y1 - y2)^2)
        // weight1 = (1/dist1) / (1/dist1 + 1/dist2 + 1/dist3)
        // prediction = weight1*brightness1 + weight2*brightness2 + weight3*brightness
        assert_eq!(44, controller.predict(50, Some(50)));
    }
}