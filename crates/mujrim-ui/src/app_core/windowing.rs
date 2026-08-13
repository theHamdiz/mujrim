//! Host window policy: compositor decorations on Linux/Wayland, CSD elsewhere.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPolicy {
    pub show_titlebar: bool,
    pub undecorated: bool,
    pub undecorated_shadow: bool,
    pub client_window_controls: bool,
    pub client_resize_edges: bool,
}

impl WindowPolicy {
    pub const LINUX_WAYLAND: Self = Self {
        show_titlebar: true,
        undecorated: false,
        undecorated_shadow: false,
        client_window_controls: false,
        client_resize_edges: false,
    };

    pub const CLIENT_CHROME: Self = Self {
        show_titlebar: false,
        undecorated: true,
        undecorated_shadow: true,
        client_window_controls: true,
        client_resize_edges: true,
    };

    pub fn for_os(os: &str) -> Self {
        match os {
            "linux" | "freebsd" | "openbsd" | "netbsd" | "dragonfly" => Self::LINUX_WAYLAND,
            _ => Self::CLIENT_CHROME,
        }
    }

    pub fn current() -> Self {
        Self::for_os(std::env::consts::OS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_policy_uses_compositor_decorations() {
        let policy = WindowPolicy::for_os("linux");
        assert!(policy.show_titlebar);
        assert!(!policy.undecorated);
        assert!(!policy.client_window_controls);
        assert!(!policy.client_resize_edges);
    }

    #[test]
    fn windows_and_macos_keep_client_chrome() {
        for os in ["windows", "macos"] {
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
