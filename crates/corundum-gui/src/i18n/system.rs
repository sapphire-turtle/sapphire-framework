//! The system's preferred languages.

/// The system's preferred languages, as BCP 47 tags, most preferred first.
///
/// Android has a list of them, read from `LocaleList`; elsewhere the platform's locale
/// (or locales) as `sys-locale` reports it. Every tag parses as a language identifier and
/// uses `-`, not `_`; anything else, such as the `C` or `POSIX` locale, is dropped.
pub fn system_languages() -> Vec<String> {
    #[cfg(target_os = "android")]
    let raw = android_languages()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| sys_locale::get_locales().collect());
    #[cfg(not(target_os = "android"))]
    let raw: Vec<String> = sys_locale::get_locales().collect();
    normalise(raw)
}

/// Each tag trimmed, with `_` made `-`; empty and unparseable ones (`C`, `POSIX`) dropped.
fn normalise(raw: impl IntoIterator<Item = String>) -> Vec<String> {
    raw.into_iter()
        .map(|t| t.trim().replace('_', "-"))
        // "POSIX" is even a well-formed language subtag, so it is named.
        .filter(|t| !t.eq_ignore_ascii_case("posix"))
        .filter(|t| t.parse::<unic_langid::LanguageIdentifier>().is_ok())
        .collect()
}

/// `LocaleList.getDefault().toLanguageTags()`, split into tags; `None` when the JVM
/// cannot be reached.
#[cfg(target_os = "android")]
fn android_languages() -> Option<Vec<String>> {
    // `android_context` panics when nothing has initialised it (an app not started by
    // android-activity, say): fall back to sys-locale then.
    let ctx = std::panic::catch_unwind(ndk_context::android_context).ok()?;
    let tags = read_locale_list(&ctx).ok()?;
    Some(
        tags.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

#[cfg(target_os = "android")]
fn read_locale_list(ctx: &ndk_context::AndroidContext) -> jni::errors::Result<String> {
    use jni::JavaVM;

    // SAFETY: `ndk_context` holds the process's JavaVM, set by android-activity before
    // `android_main` runs and valid for the process's lifetime.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let tags = read_tags(&mut env);
    // A failed call leaves its Java exception pending, and the next JNI call on this
    // thread would fail with it: clear it, the caller falls back.
    if tags.is_err() && env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    tags
}

#[cfg(target_os = "android")]
fn read_tags(env: &mut jni::JNIEnv) -> jni::errors::Result<String> {
    use jni::objects::JString;
    // The thread may stay attached, so locals made here would never be freed: a frame
    // frees them on the way out.
    env.with_local_frame(16, |env| {
        let list = env
            .call_static_method(
                "android/os/LocaleList",
                "getDefault",
                "()Landroid/os/LocaleList;",
                &[],
            )?
            .l()?;
        let tags = env
            .call_method(&list, "toLanguageTags", "()Ljava/lang/String;", &[])?
            .l()?;
        env.get_string(&JString::from(tags)).map(String::from)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use unic_langid::LanguageIdentifier;

    #[test]
    fn normalise_cleans_and_drops() {
        let raw = ["en_US", " ja-JP ", "", "C", "POSIX"].map(String::from);
        assert_eq!(normalise(raw), ["en-US", "ja-JP"]);
    }

    #[test]
    fn system_languages_parse_or_are_empty() {
        for tag in system_languages() {
            assert!(!tag.contains('_'), "{tag:?} still has an underscore");
            assert!(
                tag.parse::<LanguageIdentifier>().is_ok(),
                "{tag:?} is not a language tag"
            );
        }
    }
}
