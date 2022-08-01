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

/// Types of remote package.
pub enum RemotePackageType {
    #[cfg(feature = "debian")]
    /// Debian package
    Deb,

    #[cfg(feature = "rpm")]
    /// RPM package
    Rpm,
}

/// Error type for this crate.
#[derive(thiserror::Error, Debug)]
pub enum PkgError {
    /// An error from the underlying Debian package library
    #[cfg(feature = "debian")]
    #[error("Debpkg Error")]
    DebPkgError(#[from] debpkg::Error),

    /// An error from the underlying RPM package library
    #[cfg(feature = "rpm")]
    #[error("fez Error")]
    RpmError(#[from] ::fez::RPMError),

    /// An error from the underlying HTTP client library
    #[cfg(feature = "http")]
    #[error("HTTP Error")]
    HTTPError(#[from] reqwest::Error),

    /// Failure to infer package type
    #[error("Failed to infer package type")]
    InferError,

    /// Field not found in Debian control data.
    #[cfg(feature = "debian")]
    #[error("Debian field not found: {0}")]
    DebianControlFieldNotFound(String),

    /// Package type can't be queried.
    #[error("Package type cannot be queried (inferred: {0})")]
    UnknownPackageType(String),
}

/// Trait representing a remote package.
///
/// All remote packages support these methods.
pub trait RemotePackage {
    /// Get the package type.
    fn package_type(&self) -> RemotePackageType;

    /// Get the package name.
    fn package_name(&self) -> Result<&str, PkgError>;

    /// Get the package version.
    fn package_version(&self) -> Result<&str, PkgError>;

    /// Get the package iteration. Different package types get this from
    /// different places.
    fn package_iteration(&self) -> Option<&str>;

    /// Get the package architecture.
    fn package_arch(&self) -> Result<&str, PkgError>;
}

// Include Debian package support
#[cfg(feature = "debian")]
pub mod debian;

// Include RPM package support
#[cfg(feature = "rpm")]
pub mod rpm;

/// Create a RemotePackage from a URL.
///
/// Uses a blocking tokio client to download the remote package - if
/// using this in an async environment, surround this with tokio::spawn_blocking.
#[cfg(feature = "http")]
pub fn from_url(url: &str) -> Result<Box<dyn RemotePackage>, PkgError> {
    use std::io::Read;

    let client = reqwest::blocking::Client::new();

    // Send an HTTP request for the package and get the Response.
    let response = client.get(url).send()?;

    // Read the first 1024 bytes for infer.
    let mut reader = response.take(1024);
    let mut infer_buf = vec![];
    let _ = reader
        .read_to_end(&mut infer_buf)
        .map_err(|_| PkgError::InferError)?;

    // Infer uses magic to detect file types from starting bytes.
    let ext = infer::get(&infer_buf).map(|t| t.extension());
    let is_deb = infer::archive::is_deb(&infer_buf);
    let is_rpm = infer::archive::is_rpm(&infer_buf);

    // Using a cursor and chain allows us to reconstruct the original response.
    let rsp = std::io::Cursor::new(infer_buf).chain(reader.into_inner());

    // If the feature is enabled and the package is Debian, make a Debian remote package.
    #[cfg(feature = "debian")]
    if is_deb {
        let pkg = debian::DebianRemotePackage::new_from_read(rsp)?;
        return Ok(Box::new(pkg));
    }

    // If the feature is enabled and the package is RPM, make an RPM remote package.
    #[cfg(feature = "rpm")]
    if is_rpm {
        let pkg = rpm::RpmRemotePackage::new_from_read(rsp)?;
        return Ok(Box::new(pkg));
    }

    // The package type was unknown or the necessary feature was disabled.
    // Return an error in either case.
    Err(PkgError::UnknownPackageType(
        ext.unwrap_or("unknown").to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http")]
    fn test_from_url(url: &str, package_name: &str) -> Result<(), Box<dyn std::error::Error>> {
        let package = from_url(url).expect("Failed to download package");

        println!(
            "{}:{}:{}:{}",
            package.package_name()?,
            package.package_version()?,
            package.package_arch()?,
            package.package_iteration().unwrap_or("no iteration")
        );
        assert_eq!(package.package_name()?, package_name);
        Ok(())
    }

    #[cfg(all(feature = "http", feature = "rpm"))]
    #[test]
    fn test_from_url_rpm() -> Result<(), Box<dyn std::error::Error>> {
        // Kibana is huge but should finish very quickly if just getting metadata.
        test_from_url(
            "https://artifacts.elastic.co/downloads/kibana/kibana-8.2.1-x86_64.rpm",
            "kibana",
        )
    }

    #[cfg(all(feature = "http", feature = "debian"))]
    #[test]
    fn test_from_url_deb() -> Result<(), Box<dyn std::error::Error>> {
        // Test a small Debian package.
        test_from_url(
            "http://cz.archive.ubuntu.com/ubuntu/pool/universe/d/debian-faq/debian-faq_10.1_all.deb",
            "debian-faq",
        )
    }
}
