pub mod overlay;
pub mod overlay_impl;

pub use overlay::{Handoff, Overlay, OverlayStats, Promote, Setup};
pub use overlay_impl::{
    OverlayAcceptor, OverlayError, OverlayHandoff, OverlayImpl, PeerReservation,
    PeerReservationSource, PeerReservationTable,
};
