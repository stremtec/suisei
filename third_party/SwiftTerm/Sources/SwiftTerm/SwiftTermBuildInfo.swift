// This file is generated. Do not edit it.

/// Source-control information for this SwiftTerm build.
public enum SwiftTermBuildInfo {
    /// The Git branch, if the build uses a branch checkout.
    public static let branch: String? = "main"

    /// The exact Git tag for the current commit, if one is available.
    public static let tag: String? = nil

    /// The full Git commit identifier, if one is available.
    public static let commit: String? = "105299618ef6de56974c9d502d4346c5a74b68ce"

    /// Whether the repository had uncommitted changes during the build.
    ///
    /// This value is `nil` when neither Git nor an environment fallback
    /// can determine the worktree state.
    public static let hasUncommittedChanges: Bool? = false

    /// A value suitable for display in logs and diagnostic output.
    ///
    /// This value uses the exact tag when available. Otherwise, it uses the
    /// first 12 characters of the commit identifier. It is `"unknown"` when
    /// the package has no Git information. Modified builds have a `-modified`
    /// suffix.
    public static let version: String = {
        let base = tag ?? commit.map { String($0.prefix(12)) } ?? "unknown"
        return hasUncommittedChanges == true ? "\(base)-modified" : base
    }()
}
