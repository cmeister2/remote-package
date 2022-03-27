//! A simple crate to query remote packages for information.
#![deny(
    missing_docs,
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results
)]

/// Error type for this crate.
#[derive(thiserror::Error, Debug)]
pub enum PkgError {
    /// An error from the underlying Debian package library
    #[cfg(feature = "debian")]
    #[error("Debpkg Error")]
    DebPkgError(#[from] debpkg::Error),

    /// An error from the underlying RPM package library
    #[cfg(feature = "rpm")]
    #[error("rpm-rs Error")]
    RpmError(#[from] ::rpm::RPMError),

    /// An error from the underlying HTTP client library
    #[cfg(feature = "http")]
    #[error("HTTP Error")]
    HTTPError(#[from] reqwest::Error),
}

/// Trait representing a remote package.
///
/// All remote packages support these methods.
pub trait RemotePackage {
    /// Get the package name according to the package itself.
    fn package_name(&self) -> Result<&str, PkgError>;
}

// Include Debian package support
#[cfg(feature = "debian")]
pub mod debian;

// Include RPM package support
#[cfg(feature = "rpm")]
pub mod rpm;
