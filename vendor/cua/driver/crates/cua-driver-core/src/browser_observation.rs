//! Browser observation integration for the trusted Rust host.
//!
//! `BrowserReadEpisode` executes the retained CUA engine without registering
//! tools. The host supplies independently proved process, endpoint, native
//! geometry and exact compositor/client generation evidence. Caller tool JSON
//! cannot select the episode owner. Consumer profiles require the still-refused
//! explicit grant path; observation never prepares or enables an endpoint.
pub use crate::browser::read::{
    BoundNavigationEndpoint, BrowserElement, BrowserElementRef, BrowserReadCloseHandle, BrowserReadEpisode,
    BrowserReadObservation, ExactWindowSource, OwnedBrowserSource,
};
pub use crate::embedded::ExactWindow;

/// Host-side OS proof interface. These adapter records are kept behind Dojo's
/// public computer contract; no CDP connection or BrowserEngine is exposed.
pub mod host {
    pub use crate::browser::platform::{
        BrowserConsentOutcome, BrowserConsentRequest, BrowserPlatform, BrowserVisualAction,
        ExistingProfileSetupOutcome, ExistingProfileSetupRequest, IsolatedBrowserProcess,
        PrepareAction, PrepareOutcome, PrepareRequest, PrepareSideEffects,
    };
    pub use crate::browser::refusal::{BrowserRefusal, BrowserRefusalCode};
    pub use crate::browser::types::{
        BrowserClassification, BrowserEngineFamily, BrowserProcessRole, BrowserProduct,
        EndpointOwnershipMethod, EndpointOwnershipProof, EndpointTransport, NativeOwnershipMethod,
        NativeOwnershipProof, NativeWindowInfo, OwnedEndpoint, ProcessFingerprint, Rect,
    };
}

/// Production Niri/Linux evidence adapter.
pub mod niri {
    pub use crate::browser::niri::{
        linux_process_start, BrowserConfig, NiriBrowserPlatform, NiriWindowDescriptor,
        NiriWindowSource,
    };
}
