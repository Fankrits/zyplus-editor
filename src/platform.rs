//! The operating system the app is running on, as a plain value.
//!
//! Code that differs per OS takes an `Os` parameter (and the real call site
//! passes `Os::current()`), so the Windows and Linux branches are ordinary
//! functions that unit tests exercise on any machine, including macOS.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub const fn current() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_matches_the_build_target() {
        #[cfg(target_os = "macos")]
        assert_eq!(Os::current(), Os::Mac);
        #[cfg(target_os = "windows")]
        assert_eq!(Os::current(), Os::Windows);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        assert_eq!(Os::current(), Os::Linux);
    }
}
