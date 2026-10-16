//! Android's half of [`platform`](super): the clipboard, through JNI.

use jni::JavaVM;
use jni::objects::{JObject, JString};

/// The primary clip's text, coerced as Android's own paste would, or `None`.
pub(super) fn clipboard_text() -> Option<String> {
    match read_clipboard() {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!("could not read the clipboard: {err}");
            None
        }
    }
}

fn read_clipboard() -> jni::errors::Result<Option<String>> {
    let ctx = ndk_context::android_context();
    // SAFETY: `ndk_context` holds the process's JavaVM and the activity, set by
    // android-activity before `android_main` runs and valid for the process's lifetime.
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let mut env = vm.attach_current_thread()?;

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
}
