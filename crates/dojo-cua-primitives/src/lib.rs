//! Unchanged CUA primitives, extracted at revision
//! `88aff7e5424e9ed45b93ded85eb4111472ca5190`.
//!
//! Copyright (c) 2025 Cua AI, Inc. Upstream modules and their embedded tests
//! remain MIT-licensed; see LICENSE.md and ../../vendor/cua/ledger.json.
//! This module root is Dojo integration, not an upstream production dispatch.
//! Capture identities are not authorization; consumers must supply session,
//! geometry and human-priority admission. CDP transport is not a browser adapter.

pub mod capture_registry;
pub mod element_frame;
pub mod image_utils;
pub mod window_inspection;

pub mod browser {
    pub mod cdp_ws;
    #[cfg(test)]
    mod mock_cdp;
}

pub mod element_query;
pub mod walk_budget;
pub mod text_sanitize;
pub mod key_pacing;
