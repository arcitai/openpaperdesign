//! One binding's admission and writer fence. The launcher transfers ownership only after
//! this fence has completed; HTTP sessions and stdio clones share it with browser saves.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

const PREPARED: u8 = 0;
const ACTIVE: u8 = 1;
const RETIRING: u8 = 2;
const RETIRED: u8 = 3;

pub(crate) const UNAVAILABLE: &str =
    "this project binding is inactive or retired; the call was not run and nothing was written";

#[derive(Default)]
pub(crate) struct Lifecycle {
    phase: AtomicU8,
    pub write: Arc<tokio::sync::Mutex<()>>,
    #[cfg(test)]
    pub retiring: tokio::sync::Notify,
}

impl Lifecycle {
    pub fn check_active(&self) -> Result<(), String> {
        if self.phase.load(Ordering::Acquire) == ACTIVE {
            Ok(())
        } else {
            Err(UNAVAILABLE.into())
        }
    }

    /// Only a privately owned, fully prepared server can be activated. No callers can yet
    /// hold its writer lock or change its held pages.
    pub fn activate(&self, held: &crate::HeldPages) {
        assert_eq!(self.phase.load(Ordering::Acquire), PREPARED);
        clear_layout(held);
        self.phase.store(ACTIVE, Ordering::Release);
    }

    pub async fn retire(&self, held: &crate::HeldPages) {
        // Close admission before waiting for a writer. A call that already passed the
        // check under the writer lock completes, including rollback and acknowledgement.
        let _ = self
            .phase
            .compare_exchange(ACTIVE, RETIRING, Ordering::AcqRel, Ordering::Acquire);
        #[cfg(test)]
        self.retiring.notify_one();
        let _write = self.write.lock().await;
        // Idempotent: a retained old handle must not clear a later binding's same-id keys.
        if self.phase.load(Ordering::Acquire) == RETIRING {
            clear_layout(held);
            self.phase.store(RETIRED, Ordering::Release);
        }
    }
}

fn clear_layout(held: &crate::HeldPages) {
    let held = held.lock().unwrap();
    paper_rs::mcp::exec::with_no_stretch(|s| s.retain(|(p, _)| !held.contains(p)));
}
