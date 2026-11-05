//! The process's limit on open files, raised as far as the bridge needs.
//!
//! macOS starts a process with a soft limit of 256 descriptors. The bridge spends them on
//! every app server's control and data connections, on iroh's sockets and on the files it
//! reads, and a network change — waking up on another network — makes many of those at
//! once. Reaching the limit is worse than failed opens here: with no descriptor left, iroh's
//! interface watcher (`netdev`) cannot read the system's interface list and falls back to
//! `SCNetworkInterfaceGetLocalizedDisplayName`, which dereferences a null number formatter
//! when it cannot open its own data, and the whole process dies with SIGSEGV.

/// The soft limit the bridge asks for: far above what it uses. It is also macOS's
/// `OPEN_MAX`, above which `setrlimit` refuses a soft limit even when the hard one is
/// unlimited.
#[cfg(unix)]
const WANTED: libc::rlim_t = 10_240;

/// Raise the soft limit on open files to [`WANTED`], or to the hard limit when that is lower.
///
/// Returns the limit in force afterwards, or `None` when it could not be read. A limit
/// already at or above [`WANTED`] is left as it is.
#[cfg(unix)]
pub(crate) fn raise() -> Option<libc::rlim_t> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a valid, writable `rlimit`.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) } != 0 {
        return None;
    }
    let wanted = WANTED.min(limit.rlim_max);
    if limit.rlim_cur >= wanted {
        return Some(limit.rlim_cur);
    }
    let raised = libc::rlimit {
        rlim_cur: wanted,
        rlim_max: limit.rlim_max,
    };
    // SAFETY: `raised` is a valid `rlimit` that keeps the hard limit as it was.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raised) } != 0 {
        return Some(limit.rlim_cur);
    }
    Some(wanted)
}

/// Elsewhere there is no such limit to raise.
#[cfg(not(unix))]
pub(crate) fn raise() -> Option<u64> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn the_limit_is_raised_and_raising_again_keeps_it() {
        let first = super::raise().expect("the limit can be read");
        assert!(first >= 256, "a soft limit of {first}");
        assert_eq!(super::raise(), Some(first));
    }
}
