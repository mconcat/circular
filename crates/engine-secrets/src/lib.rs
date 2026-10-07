#![forbid(unsafe_code)]

mod environment;
mod secret_vault;

#[doc(hidden)]
pub mod engine_integration {
    use super::{InMemorySecretBytes, SecretVault};

    #[must_use]
    pub fn vault_contains_resource(vault: &SecretVault, raw_name: &str) -> bool {
        let Some(name) = super::environment::ResourceName::parse(raw_name) else {
            return false;
        };
        vault.resolve(&name).is_some()
    }

    #[must_use]
    pub fn vault_action_material<'vault>(
        vault: &'vault SecretVault,
        raw_name: &str,
    ) -> Option<&'vault [u8]> {
        let name = super::environment::ResourceName::parse(raw_name)?;
        vault
            .resolve(&name)
            .map(InMemorySecretBytes::material_for_action)
    }
}

pub use secret_vault::{InMemorySecretBytes, SecretVault, SecretVaultError};
