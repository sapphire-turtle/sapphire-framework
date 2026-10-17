//! Android's half of [`platform`](super): the clipboard, through JNI.

use jni::JavaVM;
use jni::objects::{JObject, JString};

/// The primary clip's text, coerced as Android's own paste would, or `None`.
pub(super) fn clipboard_text() -> Option<String> {
    match read_clipboard() {
        Ok(text) => text,
        Err(err) => {
            clear_exception();
            tracing::warn!("could not read the clipboard: {err}");
            None
        }
    }
}

/// The activity whose window the app draws in, set by the mobile host: `ndk_context`
/// holds the *application* context, which has no window.
static ACTIVITY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Remember the activity (`AndroidApp::activity_as_ptr`) for [`window_insets`].
pub(crate) fn set_activity(activity: *mut std::ffi::c_void) {
    ACTIVITY.store(activity as usize, std::sync::atomic::Ordering::SeqCst);
}

/// A failed call leaves a Java exception pending, and the next JNI call with one pending
/// aborts the process: clear it.
fn clear_exception() {
    let ctx = ndk_context::android_context();
    // SAFETY: as in `read_clipboard`.
    if let Ok(vm) = unsafe { JavaVM::from_raw(ctx.vm().cast()) }
        && let Ok(env) = vm.attach_current_thread()
        && env.exception_check().unwrap_or(false)
    {
        let _ = env.exception_clear();
    }
}

fn read_clipboard() -> jni::errors::Result<Option<String>> {
    let ctx = ndk_context::android_context();
    // SAFETY: `ndk_context` holds the process's JavaVM and the application context, set by
    // android-activity before `android_main` runs and valid for the process's lifetime.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let mut env = vm.attach_current_thread()?;
    // The thread stays attached for the activity's life, so locals made here would
    // never be freed: a frame frees them on the way out.
    env.with_local_frame(32, |env| {
        let name = env.new_string("clipboard")?;
        let manager = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[(&name).into()],
            )?
            .l()?;
        if manager.is_null() {
            return Ok(None);
        }
        let clip = env
            .call_method(
                &manager,
                "getPrimaryClip",
                "()Landroid/content/ClipData;",
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?;
        if count < 1 {
            return Ok(None);
        }
        let item = env
            .call_method(
                &clip,
                "getItemAt",
                "(I)Landroid/content/ClipData$Item;",
                &[0.into()],
            )?
            .l()?;
        let text = env
            .call_method(
                &item,
                "coerceToText",
                "(Landroid/content/Context;)Ljava/lang/CharSequence;",
                &[(&activity).into()],
            )?
            .l()?;
        if text.is_null() {
            return Ok(None);
        }
        let text = env
            .call_method(&text, "toString", "()Ljava/lang/String;", &[])?
            .l()?;
        let text: String = env.get_string(&JString::from(text))?.into();
        Ok(Some(text))
    })
}

/// The system bars' and the keyboard's insets in physical pixels — left, top, right, bottom —
/// or `None` where the platform does not report them.
pub(crate) fn window_insets() -> Option<[i32; 4]> {
    match read_insets() {
        Ok(insets) => insets,
        Err(err) => {
            clear_exception();
            tracing::debug!("could not read the window insets: {err}");
            None
        }
    }
}

fn read_insets() -> jni::errors::Result<Option<[i32; 4]>> {
    let activity = ACTIVITY.load(std::sync::atomic::Ordering::SeqCst);
    if activity == 0 {
        return Ok(None);
    }
    let ctx = ndk_context::android_context();
    // SAFETY: the VM as in `read_clipboard`; the activity is the global reference
    // android-activity holds for as long as the activity lives, which outlasts the window
    // that asks.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let activity = unsafe { JObject::from_raw(activity as jni::sys::jobject) };
    let mut env = vm.attach_current_thread()?;
    // The thread stays attached for the activity's life, so locals made here would
    // never be freed: a frame frees them on the way out.
    env.with_local_frame(32, |env| {
        let window = env
            .call_method(&activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        let insets = env
            .call_method(
                &decor,
                "getRootWindowInsets",
                "()Landroid/view/WindowInsets;",
                &[],
            )?
            .l()?;
        if insets.is_null() {
            return Ok(None);
        }
        let sdk = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;
        if sdk >= 30 {
            // WindowInsets.Type.systemBars() | ime() | displayCutout()
            let bars = env
                .call_static_method("android/view/WindowInsets$Type", "systemBars", "()I", &[])?
                .i()?;
            let ime = env
                .call_static_method("android/view/WindowInsets$Type", "ime", "()I", &[])?
                .i()?;
            let cutout = env
                .call_static_method(
                    "android/view/WindowInsets$Type",
                    "displayCutout",
                    "()I",
                    &[],
                )?
                .i()?;
            let rect = env
                .call_method(
                    &insets,
                    "getInsets",
                    "(I)Landroid/graphics/Insets;",
                    &[(bars | ime | cutout).into()],
                )?
                .l()?;
            let mut side = |name: &str| env.get_field(&rect, name, "I").and_then(|v| v.i());
            Ok(Some([
                side("left")?,
                side("top")?,
                side("right")?,
                side("bottom")?,
            ]))
        } else {
            let mut side = |name: &str| {
                env.call_method(&insets, name, "()I", &[])
                    .and_then(|v| v.i())
            };
            Ok(Some([
                side("getSystemWindowInsetLeft")?,
                side("getSystemWindowInsetTop")?,
                side("getSystemWindowInsetRight")?,
                side("getSystemWindowInsetBottom")?,
            ]))
        }
    })
}
