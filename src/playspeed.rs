//! How fast a USB video really plays, and so how many pictures per second
//! reach the screen.
//!
//! mpv shows every frame (`--framedrop=no`) and, without an audio track, just
//! slows its own clock when the Pi cannot keep up. So its playback position
//! advances by less than one second per second, and the pictures on the screen
//! are that factor times the clip's frame rate. Measured on the test Pi on
//! 2026-11-07: 0.592 x 59.94 fps = 35.5, and the DRM vblank tracepoint counted
//! 35.5 page flips per second (FINDINGS.MD, "USB vs. SD card, output size and
//! 4K60"). mpv's own `estimated-vf-fps` is useless for this: it follows the
//! frame timestamps and reads 59.94 whatever the screen gets.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Readings average over this much wall time, so one slow poll does not make
/// the number jump.
const WINDOW: Duration = Duration::from_secs(4);
/// No reading before the samples span at least this long (a shorter span is
/// mostly timing noise).
const MIN_SPAN: Duration = Duration::from_millis(1500);
/// mpv plays at 1.0x, so the position can never legitimately advance faster
/// than this multiple of the wall time; more means someone seeked.
const MAX_SPEED: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Clip seconds played per second of wall time: 1.0 is real time, 0.59 is
    /// slow motion, 0.0 is paused.
    pub speed: f64,
    /// New pictures per second on the screen; `None` while the clip's own frame
    /// rate is unknown.
    pub fps: Option<f64>,
}

#[derive(Default)]
pub struct PlaySpeed {
    samples: VecDeque<(Instant, f64)>,
}

impl PlaySpeed {
    /// Forget everything measured so far (new clip, stop, seek).
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    /// Feed one poll of mpv: `position` is its `time-pos`, `clip_fps` its
    /// `container-fps`. Returns a reading once enough time has passed since the
    /// last reset; `None` means "still measuring" (or mpv did not answer).
    pub fn update(
        &mut self,
        now: Instant,
        position: Option<f64>,
        paused: bool,
        clip_fps: Option<f64>,
    ) -> Option<Reading> {
        let Some(position) = position.filter(|p| p.is_finite()) else {
            self.samples.clear();
            return None;
        };
        if paused {
            self.samples.clear();
            return Some(Reading {
                speed: 0.0,
                fps: Some(0.0),
            });
        }
        if let Some(&(at, last)) = self.samples.back() {
            let wall = now.saturating_duration_since(at).as_secs_f64();
            let advanced = position - last;
            // A jump back (seek, replay) or one faster than any playback is a
            // seek, not a speed.
            if advanced < -0.05 || advanced > wall * MAX_SPEED + 0.5 {
                self.samples.clear();
            }
        }
        self.samples.push_back((now, position));
        while self.samples.len() > 2
            && self
                .samples
                .front()
                .is_some_and(|&(at, _)| now.saturating_duration_since(at) > WINDOW)
        {
            self.samples.pop_front();
        }
        let &(first_at, first) = self.samples.front()?;
        let span = now.saturating_duration_since(first_at);
        if span < MIN_SPAN {
            return None;
        }
        let speed = (position - first) / span.as_secs_f64();
        Some(Reading {
            speed,
            fps: clip_fps
                .filter(|f| f.is_finite() && *f > 0.0)
                .map(|f| speed * f),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLIP_FPS: Option<f64> = Some(59.94);

    /// Polls once a second for `seconds` seconds of a clip playing at `speed`.
    fn play(
        p: &mut PlaySpeed,
        t0: Instant,
        from: f64,
        seconds: u32,
        speed: f64,
    ) -> Option<Reading> {
        let mut last = None;
        for n in 0..=seconds {
            last = p.update(
                t0 + Duration::from_secs(n.into()),
                Some(from + speed * f64::from(n)),
                false,
                CLIP_FPS,
            );
        }
        last
    }

    #[test]
    fn steady_slow_motion_is_measured_and_gives_the_screen_fps() {
        let mut p = PlaySpeed::default();
        let r = play(&mut p, Instant::now(), 20.0, 6, 0.592).unwrap();
        assert!((r.speed - 0.592).abs() < 1e-9);
        assert!((r.fps.unwrap() - 35.48).abs() < 0.05);
    }

    #[test]
    fn real_time_is_one_x_and_the_clip_rate() {
        let mut p = PlaySpeed::default();
        let r = play(&mut p, Instant::now(), 0.0, 5, 1.0).unwrap();
        assert!((r.speed - 1.0).abs() < 1e-9);
        assert!((r.fps.unwrap() - 59.94).abs() < 1e-6);
    }

    #[test]
    fn no_reading_before_enough_time_has_passed() {
        let mut p = PlaySpeed::default();
        let t0 = Instant::now();
        assert_eq!(p.update(t0, Some(5.0), false, CLIP_FPS), None);
        assert_eq!(
            p.update(t0 + Duration::from_secs(1), Some(5.6), false, CLIP_FPS),
            None
        );
        assert!(
            p.update(t0 + Duration::from_secs(2), Some(6.2), false, CLIP_FPS)
                .is_some()
        );
    }

    #[test]
    fn the_window_follows_a_change_of_speed() {
        let mut p = PlaySpeed::default();
        let t0 = Instant::now();
        play(&mut p, t0, 0.0, 6, 0.5);
        // Speeds up to 1.0x from second 6 on; ten seconds later only that is left.
        let mut last = None;
        for n in 7..=16_u32 {
            let pos = 3.0 + f64::from(n - 6);
            last = p.update(
                t0 + Duration::from_secs(n.into()),
                Some(pos),
                false,
                CLIP_FPS,
            );
        }
        assert!((last.unwrap().speed - 1.0).abs() < 1e-9);
    }

    #[test]
    fn paused_is_zero_and_restarts_the_measurement() {
        let mut p = PlaySpeed::default();
        let t0 = Instant::now();
        play(&mut p, t0, 0.0, 4, 0.6);
        let paused = p.update(t0 + Duration::from_secs(5), Some(3.0), true, CLIP_FPS);
        assert_eq!(
            paused,
            Some(Reading {
                speed: 0.0,
                fps: Some(0.0)
            })
        );
        // After resuming it has to measure again instead of averaging in the pause.
        assert_eq!(
            p.update(t0 + Duration::from_secs(6), Some(3.0), false, CLIP_FPS),
            None
        );
        let r = p
            .update(t0 + Duration::from_secs(8), Some(4.2), false, CLIP_FPS)
            .unwrap();
        assert!((r.speed - 0.6).abs() < 1e-9);
    }

    #[test]
    fn a_seek_is_not_a_speed() {
        let mut p = PlaySpeed::default();
        let t0 = Instant::now();
        play(&mut p, t0, 10.0, 4, 0.6);
        // Forward by a minute in one second, then back to the start.
        assert_eq!(
            p.update(t0 + Duration::from_secs(5), Some(70.0), false, CLIP_FPS),
            None
        );
        assert_eq!(
            p.update(t0 + Duration::from_secs(6), Some(0.5), false, CLIP_FPS),
            None
        );
        let r = p
            .update(t0 + Duration::from_secs(8), Some(1.7), false, CLIP_FPS)
            .unwrap();
        assert!((r.speed - 0.6).abs() < 1e-9);
    }

    #[test]
    fn an_unanswered_poll_drops_the_measurement() {
        let mut p = PlaySpeed::default();
        let t0 = Instant::now();
        play(&mut p, t0, 0.0, 4, 0.6);
        assert_eq!(
            p.update(t0 + Duration::from_secs(5), None, false, CLIP_FPS),
            None
        );
        assert_eq!(
            p.update(t0 + Duration::from_secs(6), Some(3.6), false, CLIP_FPS),
            None
        );
    }

    #[test]
    fn unknown_clip_rate_still_gives_the_speed() {
        let mut p = PlaySpeed::default();
        let t0 = Instant::now();
        let mut last = None;
        for n in 0..=3 {
            last = p.update(
                t0 + Duration::from_secs(n),
                Some(n as f64 * 0.75),
                false,
                None,
            );
        }
        let r = last.unwrap();
        assert!((r.speed - 0.75).abs() < 1e-9);
        assert_eq!(r.fps, None);
    }
}
