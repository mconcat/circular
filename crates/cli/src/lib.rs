#![forbid(unsafe_code)]

pub mod daemon_registration;
mod location_selection;
pub mod owner_local_client;
pub mod shell;

pub use location_selection::{
    InstallLocationCatalog, InstallLocationRef, InstallLocationRegistration,
    InstallLocationSelectionError, LocationCatalogError, LocationSelectionError,
    StateLocationCatalog, StateLocationRef, StateLocationRegistration, StateLocationSelectionError,
    select_install, select_state,
};
