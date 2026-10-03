//! Prepare one block view before the user opens it.
//!
//! A hover that rests on an INSERT, or a selection that is exactly one
//! editable INSERT, asks the app to tessellate that view on a background
//! thread. Nothing here changes the picture, the status line, or the cursor.

use std::sync::mpsc::Receiver;

use cad_core::EntityId;
use eframe::egui;

use crate::app::{ViewKey, ViewTessMsg};

/// Pointer must rest this long before a hover starts a prefetch.
pub(crate) const PREFETCH_HOVER_DWELL: f64 = 0.25;

const POINTER_SLOP_PX: f32 = 1.0;

// ------------------------------------------------------------
// Enum: HoverQuery
// Purpose: What the viewport should do with the resting pointer.
// ------------------------------------------------------------
pub(crate) enum HoverQuery {
    Idle,
    /// Wake once, after this many seconds, to test the dwell.
    Wait(f64),
    /// The pointer has rested. Pick one entity, then stop until it moves.
    Pick,
}

// ------------------------------------------------------------
// Enum: PrefetchReadiness
// Purpose: Whether a background block view may start on this frame.
// ------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefetchReadiness {
    Ready,
    /// Busy with a command, open, save, or another picture. Keep the target.
    Later,
    /// Dynamic drawings and nested edits are not prepared ahead of time.
    Never,
}

// ------------------------------------------------------------
// Type: PrefetchJob
// Purpose: The single block view currently being built off the UI thread.
// ------------------------------------------------------------
pub(crate) struct PrefetchJob {
    pub(crate) key: ViewKey,
    pub(crate) job: u64,
    pub(crate) rx: Receiver<Result<ViewTessMsg, String>>,
}

// ------------------------------------------------------------
// Type: BlockPrefetch
// Purpose: One running job, one newer target, and the hover dwell.
// ------------------------------------------------------------
#[derive(Default)]
pub(crate) struct BlockPrefetch {
    pub(crate) running: Option<PrefetchJob>,
    waiting: Option<EntityId>,
    hover_anchor: Option<egui::Pos2>,
    hover_since: Option<f64>,
    hover_resolved: bool,
}

impl BlockPrefetch {
    pub(crate) fn is_running(&self, instance_id: EntityId, generation: u64) -> bool {
        self.running.as_ref().is_some_and(|job| {
            job.key.generation == generation
                && job.key.frames.len() == 1
                && job.key.frames[0] == instance_id
        })
    }

    pub(crate) fn set_waiting(&mut self, instance_id: EntityId) {
        self.waiting = Some(instance_id);
    }

    pub(crate) fn waiting(&self) -> Option<EntityId> {
        self.waiting
    }

    pub(crate) fn clear_waiting(&mut self) {
        self.waiting = None;
    }

    pub(crate) fn clear_hover(&mut self) {
        self.hover_anchor = None;
        self.hover_since = None;
        self.hover_resolved = false;
    }

    /// Track a still pointer. Picking happens once per rest, not every frame.
    pub(crate) fn observe_pointer(&mut self, pos: Option<egui::Pos2>, time: f64) -> HoverQuery {
        let Some(pos) = pos else {
            self.clear_hover();
            return HoverQuery::Idle;
        };
        let moved = self
            .hover_anchor
            .map(|anchor| anchor.distance(pos) > POINTER_SLOP_PX)
            .unwrap_or(true);
        if moved {
            self.hover_anchor = Some(pos);
            self.hover_since = Some(time);
            self.hover_resolved = false;
            return HoverQuery::Wait(PREFETCH_HOVER_DWELL);
        }
        if self.hover_resolved {
            return HoverQuery::Idle;
        }
        let since = self.hover_since.unwrap_or(time);
        let elapsed = time - since;
        if elapsed < PREFETCH_HOVER_DWELL {
            return HoverQuery::Wait(PREFETCH_HOVER_DWELL - elapsed);
        }
        self.hover_resolved = true;
        HoverQuery::Pick
    }
}
