use debpkg::DebPkg;
use std::io::Read;

#[derive(thiserror::Error, Debug)]
pub enum PkgError {
    #[error("HTTP Error")]
    HTTPError(#[from] reqwest::Error),

    #[error("Debpkg Error")]
    DebPkgError(#[from] debpkg::Error),
}

pub trait RemotePackage {
    fn name(&self) -> &str;
}

pub struct DebianRemotePackage {
    control: debpkg::Control,
}

impl DebianRemotePackage {
    pub fn new_from_url(url: &str) -> Result<Self, PkgError> {
        let client = reqwest::blocking::Client::new();

        // Send an HTTP request for the package and get the Response.
        let response = client.get(url).send()?;

        // blocking::Response impls Read, so we can pass it to debpkg.
        let pkg = debpkg::DebPkg::parse(response)?;

        // Pass the package to the general constructor
        Self::new_from_control(pkg)
    }

    pub fn new_from_control<T: Read>(mut pkg: DebPkg<T>) -> Result<Self, PkgError> {
        // Get the control archive from the package.
        let archive = pkg.control()?;

        // Parse the control information.
        let control = debpkg::Control::extract(archive)?;

        Ok(Self { control })
    }
}

impl RemotePackage for DebianRemotePackage {
    fn name(&self) -> &str {
        self.control.name()
    }
}
