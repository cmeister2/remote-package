//! Support for remote Debian packages

use std::io::Read;

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
    /// Uses a blocking HTTP client. In an async environment, call this from
    /// `tokio::task::spawn_blocking`.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails, the server returns an error
    /// status, or the package control data cannot be read or parsed.
    #[cfg(feature = "http")]
    pub fn new_from_url(url: &str) -> Result<Self, PkgError> {
        let client = reqwest::blocking::Client::new();

        // Send an HTTP request for the package and get the Response.
        let response = client.get(url).send()?.error_for_status()?;

        // Response impls Read, so pass it to new_from_read().
        Self::new_from_read(response)
    }

    /// Attempts to create a `DebianRemotePackage` from a reader.
    ///
    /// # Errors
    ///
    /// Returns an error if the package or its control archive cannot be read or
    /// parsed.
    pub fn new_from_read<R: Read>(reader: R) -> Result<Self, PkgError> {
        let pkg = DebPkg::parse(reader)?;

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
    //! Debian control parsing and loopback HTTP regressions.

    use super::*;
    use crate::tests::debian_package_bytes;

    /// Parse all exposed fields, including the final Debian revision component.
    #[test]
    fn reads_control_metadata() {
        let bytes = debian_package_bytes(
            "Package: metadata-test\nVersion: 1.2.3-pre-4\nArchitecture: all\nDescription: fixture\n",
        );
        let package = DebianRemotePackage::new_from_read(bytes.as_slice()).unwrap();
        assert!(matches!(
            package.package_type(),
            crate::RemotePackageType::Deb
        ));
        assert_eq!(package.package_name().unwrap(), "metadata-test");
        assert_eq!(package.package_version().unwrap(), "1.2.3-pre-4");
        assert_eq!(package.package_iteration(), Some("4"));
        assert_eq!(package.package_arch().unwrap(), "all");
    }

    /// Upstream-only versions have no Debian revision.
    #[test]
    fn handles_missing_revision() {
        let bytes = debian_package_bytes(
            "Package: metadata-test\nVersion: 1.2.3\nArchitecture: all\nDescription: fixture\n",
        );
        let package = DebianRemotePackage::new_from_read(bytes.as_slice()).unwrap();
        assert_eq!(package.package_iteration(), None);
    }

    /// Missing architecture data produces the crate's field-specific error.
    #[test]
    fn reports_missing_architecture() {
        let bytes =
            debian_package_bytes("Package: metadata-test\nVersion: 1.2.3\nDescription: fixture\n");
        let package = DebianRemotePackage::new_from_read(bytes.as_slice()).unwrap();
        assert!(matches!(
            package.package_arch(),
            Err(PkgError::DebianControlFieldNotFound(field)) if field == "Architecture"
        ));
    }

    /// A truncated archive propagates its underlying package error.
    #[test]
    fn rejects_truncated_package() {
        assert!(matches!(
            DebianRemotePackage::new_from_read(&b"!<arch>\n"[..]),
            Err(PkgError::DebPkgError(_))
        ));
    }

    /// The direct HTTP constructor reads the same metadata as the reader API.
    #[cfg(feature = "http")]
    #[test]
    fn test_package() {
        let bytes = debian_package_bytes(
            "Package: metadata-test\nVersion: 1.2.3-4\nArchitecture: all\nDescription: HTTP fixture\n",
        );
        let (url, server) = crate::tests::serve_package(bytes);
        let package = DebianRemotePackage::new_from_url(&url).unwrap();
        assert_eq!(package.package_name().unwrap(), "metadata-test");
        assert_eq!(package.package_version().unwrap(), "1.2.3-4");
        assert_eq!(package.package_arch().unwrap(), "all");
        assert_eq!(package.package_iteration(), Some("4"));
        server.join().unwrap();
    }

    /// The direct constructor preserves HTTP status failures.
    #[cfg(feature = "http")]
    #[test]
    fn reports_http_status_errors() {
        let (url, server) = crate::tests::serve_response("404 Not Found", Vec::new());
        assert!(matches!(
            DebianRemotePackage::new_from_url(&url),
            Err(PkgError::HTTPError(error)) if error.status() == Some(reqwest::StatusCode::NOT_FOUND)
        ));
        server.join().unwrap();
    }
}
