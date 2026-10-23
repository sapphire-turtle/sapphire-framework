//! The system's preferred languages.

/// The system's preferred languages, as BCP 47 tags, most preferred first.
///
/// Android has a list of them, read from `LocaleList`; elsewhere the platform's locale
/// (or locales) as `sys-locale` reports it. Tags use `-`, not `_`, and none is empty.
pub fn system_languages() -> Vec<String> {
    #[cfg(target_os = "android")]
    let raw = android_languages()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| sys_locale::get_locales().collect());
    #[cfg(not(target_os = "android"))]
    let raw: Vec<String> = sys_locale::get_locales().collect();
    raw.into_iter()
        .map(|t| t.trim().replace('_', "-"))
        .filter(|t| !t.is_empty())
        .collect()
}

/// `LocaleList.getDefault().toLanguageTags()`, split into tags; `None` when the JVM
/// cannot be reached.
#[cfg(target_os = "android")]
fn android_languages() -> Option<Vec<String>> {
    read_locale_list()
        .ok()
        .map(|tags| tags.split(',').map(str::to_owned).collect())
}

#[cfg(target_os = "android")]
fn read_locale_list() -> jni::errors::Result<String> {
    use jni::JavaVM;
    use jni::objects::JString;

    let ctx = ndk_context::android_context();
    // SAFETY: `ndk_context` holds the process's JavaVM, set by android-activity before
    // `android_main` runs and valid for the process's lifetime.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
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
