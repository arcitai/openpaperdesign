//! Renderer health: what to do with each surface-acquire outcome, and when the page may say
//! it is ready. Plain Rust, so it is tested on the host; `web.rs` maps wgpu's
//! `CurrentSurfaceTexture` and its error/device-lost callbacks onto it.
//!
//! Ready means: the GPU objects validated, at least one frame was actually presented, and no
//! GPU error, device loss or terminal surface error has been recorded since.

/// wgpu 30's `CurrentSurfaceTexture`, without the texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquired {
    Success,
    Suboptimal,
    Timeout,
    Occluded,
    Outdated,
    Lost,
    Validation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Draw and present; reconfigure the surface afterwards if it was suboptimal.
    Draw { reconfigure_after: bool },
    /// Nothing to draw into this frame (timeout, hidden); try again on the next one.
    Skip,
    /// The surface configuration is outdated: reconfigure, then try again next frame.
    Reconfigure,
    /// The surface can't be used any more; show the reason and stop drawing.
    Fail(String),
}

pub fn step(a: Acquired) -> Step {
    match a {
        Acquired::Success => Step::Draw {
            reconfigure_after: false,
        },
        Acquired::Suboptimal => Step::Draw {
            reconfigure_after: true,
        },
        Acquired::Timeout | Acquired::Occluded => Step::Skip,
        Acquired::Outdated => Step::Reconfigure,
        Acquired::Lost => Step::Fail("the drawing surface was lost".into()),
        Acquired::Validation => {
            Step::Fail("getting the drawing surface failed GPU validation".into())
        }
    }
}

#[derive(Debug, Default)]
pub struct Health {
    presented: u64,
    error: Option<String>,
}

impl Health {
    /// Record a terminal problem. The first one is kept (later ones are usually follow-ups).
    pub fn fail(&mut self, msg: impl Into<String>) {
        self.error.get_or_insert_with(|| msg.into());
    }

    /// A frame was presented.
    pub fn presented(&mut self) {
        self.presented += 1;
    }

    pub fn can_draw(&self) -> bool {
        self.error.is_none()
    }

    pub fn ready(&self) -> bool {
        self.error.is_none() && self.presented > 0
    }

    pub fn error(&self) -> Option<String> {
        self.error.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_outcomes() {
        assert_eq!(
            step(Acquired::Success),
            Step::Draw {
                reconfigure_after: false
            }
        );
        assert_eq!(
            step(Acquired::Suboptimal),
            Step::Draw {
                reconfigure_after: true
            }
        );
        assert_eq!(step(Acquired::Timeout), Step::Skip);
        assert_eq!(step(Acquired::Occluded), Step::Skip);
        assert_eq!(step(Acquired::Outdated), Step::Reconfigure);
        assert!(matches!(step(Acquired::Lost), Step::Fail(_)));
        assert!(matches!(step(Acquired::Validation), Step::Fail(_)));
    }

    #[test]
    fn ready_needs_a_presented_frame() {
        let mut h = Health::default();
        assert!(!h.ready(), "validated but nothing presented yet");
        // First frames skipped (e.g. a hidden tab): still not ready, still allowed to draw.
        for _ in 0..3 {
            assert_eq!(step(Acquired::Timeout), Step::Skip);
        }
        assert!(!h.ready() && h.can_draw());
        h.presented();
        assert!(h.ready());
    }

    #[test]
    fn any_recorded_error_ends_readiness_and_drawing() {
        // Before the first frame (fault injection: an uncaptured error during startup).
        let mut h = Health::default();
        h.fail("wgpu: invalid render pipeline");
        h.presented();
        assert!(!h.ready() && !h.can_draw());
        assert_eq!(h.error().as_deref(), Some("wgpu: invalid render pipeline"));

        // After being ready: device lost.
        let mut h = Health::default();
        h.presented();
        assert!(h.ready());
        h.fail("GPU device lost (Unknown): driver reset");
        h.fail("later follow-up error");
        assert!(!h.ready() && !h.can_draw());
        assert_eq!(
            h.error().as_deref(),
            Some("GPU device lost (Unknown): driver reset")
        );
    }

    #[test]
    fn terminal_surface_outcomes_fail_readiness() {
        for a in [Acquired::Lost, Acquired::Validation] {
            let mut h = Health::default();
            h.presented();
            if let Step::Fail(msg) = step(a) {
                h.fail(msg);
            }
            assert!(!h.ready(), "{a:?}");
        }
    }
}
