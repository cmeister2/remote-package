//! Support for remote Debian packages

use std::{convert::TryFrom, io::Read};

use debpkg::DebPkg;

use crate::{PkgError, RemotePackage};

/// A structure representing a remote Debian package.
#[derive(Debug)]
pub struct DebianRemotePackage {
    /// Structure containing the control portion of the remote Debian package
    control: debpkg::Control,
}

impl DebianRemotePackage {
    /// Attempts to create a `DebianRemotePackage` from a URL.
    ///
    /// Uses a blocking tokio client to download the remote package - if
    /// using this in an async environment, surround this with tokio::spawn_blocking.
    #[cfg(feature = "http")]
    pub fn new_from_url(url: &str) -> Result<Self, PkgError> {
        let client = reqwest::blocking::Client::new();

        // Send an HTTP request for the package and get the Response.
        let response = client.get(url).send()?;

        // Response impls Read, so pass it to new_from_read().
        Self::new_from_read(response)
    }

    /// Attempts to create a `DebianRemotePackage` from something that impls
    /// Read.
    pub fn new_from_read<R: Read>(reader: R) -> Result<Self, PkgError> {
        let pkg = debpkg::DebPkg::parse(reader)?;

        // Pass the package to the general constructor
        Self::try_from(pkg)
    }
}

impl<T> TryFrom<DebPkg<T>> for DebianRemotePackage
where
    T: Read,
{
    type Error = PkgError;

    fn try_from(mut pkg: DebPkg<T>) -> Result<Self, Self::Error> {
        // Get the control archive from the package.
        let archive = pkg.control()?;

        // Parse the control information.
        let control = debpkg::Control::extract(archive)?;

        Ok(Self { control })
    }
}

impl RemotePackage for DebianRemotePackage {
    fn package_type(&self) -> crate::RemotePackageType {
        crate::RemotePackageType::Deb
    }

    fn package_name(&self) -> Result<&str, PkgError> {
        // Just return the control name successfully
        Ok(self.control.name())
    }

    fn package_version(&self) -> Result<&str, PkgError> {
        Ok(self.control.version())
    }

    fn package_arch(&self) -> Result<&str, PkgError> {
        self.control
            .get("Architecture")
            .ok_or_else(|| PkgError::DebianControlFieldNotFound("Architecture".to_string()))
    }

    /// For Debian, the package iteration is the debian_revision.
    fn package_iteration(&self) -> Option<&str> {
        // Start by getting the version.
        let version = self.control.version();

        // Split the version on "-". If there's a debian_revision, it'll be
        // the matched suffix.
        version.rsplit_once('-').map(|(_prefix, suffix)| suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "http")]
    #[test]
    fn test_package() {
        // Pick a random small package from the Ubuntu focal LTS
        let url = "http://cz.archive.ubuntu.com/ubuntu/pool/universe/d/debian-faq/debian-faq_10.1_all.deb";

        let deb = DebianRemotePackage::new_from_url(url).expect("Failed to download package");
        assert_eq!(deb.package_name().unwrap(), "debian-faq");
    }
}
