//! Host window policy: client-side decorations on every platform.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPolicy {
    pub show_titlebar: bool,
    pub undecorated: bool,
    pub undecorated_shadow: bool,
    pub client_window_controls: bool,
    pub client_resize_edges: bool,
}

impl WindowPolicy {
    pub const CLIENT_CHROME: Self = Self {
        show_titlebar: false,
        undecorated: true,
        undecorated_shadow: true,
        client_window_controls: true,
        client_resize_edges: true,
    };

    pub fn for_os(_os: &str) -> Self {
        Self::CLIENT_CHROME
    }

    pub fn current() -> Self {
        Self::for_os(std::env::consts::OS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_os_uses_custom_titlebar() {
        for os in ["linux", "freebsd", "windows", "macos"] {
            let policy = WindowPolicy::for_os(os);
            assert!(!policy.show_titlebar);
            assert!(policy.undecorated);
            assert!(policy.client_window_controls);
            assert!(policy.client_resize_edges);
        }
    }

    #[test]
    fn current_policy_matches_host_os() {
        assert_eq!(
            WindowPolicy::current(),
            WindowPolicy::for_os(std::env::consts::OS)
        );
    }
}
