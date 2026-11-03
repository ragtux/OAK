//! The keys M4's git support reads. The defaults are what Fleet's Settings
//! tool showed on its Tools › Git page (batch 8, `docs/fleet-probe.md`,
//! seventh pass); the two not on a page are marked "estimate".

use crate::Settings;

impl Settings {
    /// `git.autoFetch`: "Run git fetch with a regular interval. This is used
    /// for showing incoming/outgoing commits in the Git panel." Default true.
    pub fn git_auto_fetch(&self) -> bool {
        self.bool("git.autoFetch").unwrap_or(true)
    }

    /// `git.fetchInterval`: "The time interval in minutes for git.autoFetch".
    /// Default 15.
    pub fn git_fetch_interval_min(&self) -> f64 {
        self.f64("git.fetchInterval")
            .filter(|m| *m > 0.)
            .unwrap_or(15.)
    }

    /// `git.push.confirm`: "Show confirmation on push". Default true.
    pub fn git_push_confirm(&self) -> bool {
        self.bool("git.push.confirm").unwrap_or(true)
    }

    /// `git.revert.confirm`: "Show confirmation on revert". Default true.
    pub fn git_revert_confirm(&self) -> bool {
        self.bool("git.revert.confirm").unwrap_or(true)
    }

    /// `editor.showLineStatus`: "Show changes in the gutter". Default true.
    pub fn editor_show_line_status(&self) -> bool {
        self.bool("editor.showLineStatus").unwrap_or(true)
    }

    /// `git.discoverGitManually`: whether WEEP looks for the `git` binary on
    /// the path (true) rather than through Fleet's bundled one. WEEP always
    /// uses the path; the key is read for the record. Default false (estimate).
    pub fn git_discover_git_manually(&self) -> bool {
        self.bool("git.discoverGitManually").unwrap_or(false)
    }

    /// `git.enable3WayDiffView`: the three-way view for conflicts. Default
    /// false (estimate; the AI page hid the Git page's last rows).
    pub fn git_enable_3way_diff_view(&self) -> bool {
        self.bool("git.enable3WayDiffView").unwrap_or(false)
    }

    /// `ff.expandGitDiffByDefault`: every file's diff open in the Changed
    /// Files tab, not only the one that opened it. Default false (the ninth
    /// pass showed one card open; estimate).
    pub fn ff_expand_git_diff_by_default(&self) -> bool {
        self.bool("ff.expandGitDiffByDefault").unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_defaults() {
        let s = Settings::parse("{}").unwrap();
        assert!(s.git_auto_fetch());
        assert_eq!(s.git_fetch_interval_min(), 15.);
        assert!(s.git_push_confirm());
        assert!(s.git_revert_confirm());
        assert!(s.editor_show_line_status());
        let s = Settings::parse(
            r#"{"git.autoFetch": false, "git.fetchInterval": 0, "editor.showLineStatus": false}"#,
        )
        .unwrap();
        assert!(!s.git_auto_fetch());
        assert_eq!(s.git_fetch_interval_min(), 15., "zero falls back");
        assert!(!s.editor_show_line_status());
    }
}
