//! Imported pictures stay in this browser. IndexedDB is per site visitor, so the
//! editor works without sending the file to Cloudflare. A chat may send a temporary
//! spool so see and watch can read a frame. That copy is not the media library.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::{Blob, BlobPropertyBag, IdbTransactionMode};

const DB_NAME: &str = "opencut-media";
const STORE: &str = "files";
const DB_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldMeta {
    pub id: String,
    pub name: String,
    pub content_type: String,
}

/// How a stored clip sits in IndexedDB.
/// A video copied into a Rust `Vec` fills the wasm heap, and Chrome then kills
/// the tab while speech is read (Aw, Snap, error code 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HoldBody {
    /// Blob handle. Reading the record does not copy the picture into Rust.
    Blob,
    /// Older record that stored a byte array. Wrap it once, then drop the array.
    LegacyBytes,
    Missing,
}

pub(crate) fn hold_body(has_blob: bool, has_bytes: bool) -> HoldBody {
    if has_blob {
        HoldBody::Blob
    } else if has_bytes {
        HoldBody::LegacyBytes
    } else {
        HoldBody::Missing
    }
}

#[must_use]
pub fn hold_key(project_id: &str, media_id: &str) -> String {
    format!("{project_id}/{media_id}")
}

/// A still for one clip. Same store, different key, so a reload does not upload again.
#[must_use]
pub fn poster_id(media_id: &str) -> String {
    format!("{media_id}:poster")
}

#[must_use]
pub fn is_poster_id(id: &str) -> bool {
    id.ends_with(":poster")
}

/// Library restore must not treat a cached still as another video.
#[must_use]
fn is_library_file(id: &str) -> bool {
    !id.is_empty() && !is_poster_id(id)
}

#[must_use]
pub fn project_of_key(key: &str) -> Option<&str> {
    let (project, media) = key.split_once('/')?;
    if project.is_empty() || media.is_empty() {
        None
    } else {
        Some(project)
    }
}

/// Keep a clip as a Blob. The bytes are not copied into Rust.
pub async fn save_blob(
    project_id: &str,
    media_id: &str,
    name: &str,
    content_type: &str,
    blob: &Blob,
) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (project_id, media_id, name, content_type, blob);
        return Err("a clip is stored in the browser".into());
    }
    #[cfg(target_arch = "wasm32")]
    {
        save_blob_wasm(project_id, media_id, name, content_type, blob).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn save_blob_wasm(
    project_id: &str,
    media_id: &str,
    name: &str,
    content_type: &str,
    blob: &Blob,
) -> Result<(), String> {
    let db = open_db().await?;
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
        .map_err(js_err)?;
    let store = tx.object_store(STORE).map_err(js_err)?;
    let record = js_sys::Object::new();
    set_field(&record, "project_id", &JsValue::from_str(project_id))?;
    set_field(&record, "id", &JsValue::from_str(media_id))?;
    set_field(&record, "name", &JsValue::from_str(name))?;
    set_field(&record, "content_type", &JsValue::from_str(content_type))?;
    set_field(&record, "blob", blob.as_ref())?;
    let req = store
        .put_with_key(&record, &JsValue::from_str(&hold_key(project_id, media_id)))
        .map_err(js_err)?;
    await_request(req).await?;
    Ok(())
}

/// A still is small. The video path uses [`save_blob`] so the picture never enters wasm.
#[cfg(target_arch = "wasm32")]
async fn save_file(
    project_id: &str,
    media_id: &str,
    name: &str,
    content_type: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    let parts = js_sys::Array::of1(&array);
    let opts = BlobPropertyBag::new();
    opts.set_type(content_type);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js_err)?;
    save_blob(project_id, media_id, name, content_type, &blob).await
}

pub async fn save_poster(project_id: &str, media_id: &str, bytes: &[u8]) -> Result<(), String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (project_id, media_id, bytes);
        return Ok(());
    }
    #[cfg(target_arch = "wasm32")]
    {
        if media_id.is_empty() || is_poster_id(media_id) || bytes.is_empty() {
            return Err("no still to store".into());
        }
        save_file(
            project_id,
            &poster_id(media_id),
            "poster.jpg",
            "image/jpeg",
            bytes,
        )
        .await
    }
}

pub async fn load_poster(project_id: &str, media_id: &str) -> Result<Option<Vec<u8>>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (project_id, media_id);
        return Ok(None);
    }
    #[cfg(target_arch = "wasm32")]
    {
        load_poster_wasm(project_id, media_id).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn load_poster_wasm(project_id: &str, media_id: &str) -> Result<Option<Vec<u8>>, String> {
    if project_id.is_empty() || media_id.is_empty() || is_poster_id(media_id) {
        return Ok(None);
    }
    let value = get_record(&hold_key(project_id, &poster_id(media_id))).await?;
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let blob = blob_from_record(&value)?;
    blob_bytes(&blob).await.map(Some)
}

/// Names of clips in this browser. One record at a time, so a legacy byte array
/// is wrapped and dropped before the next clip is opened.
pub async fn list_for_project(project_id: &str) -> Result<Vec<HeldMeta>, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = project_id;
        return Ok(Vec::new());
    }
    #[cfg(target_arch = "wasm32")]
    {
        list_for_project_wasm(project_id).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn list_for_project_wasm(project_id: &str) -> Result<Vec<HeldMeta>, String> {
    let keys = all_keys().await?;
    let mut out = Vec::new();
    for key in keys {
        if project_of_key(&key) != Some(project_id) {
            continue;
        }
        let record = get_record(&key).await?;
        if record.is_null() || record.is_undefined() {
            continue;
        }
        let Some(meta) = meta_from_record(&record) else {
            continue;
        };
        if !is_library_file(&meta.id) {
            continue;
        }
        let kind = classify_record(&record);
        if kind == HoldBody::Missing {
            continue;
        }
        let legacy = kind == HoldBody::LegacyBytes || has_binary_bytes(&record);
        if legacy {
            let rewritten = if let Ok(blob) = blob_from_record(&record) {
                save_blob(project_id, &meta.id, &meta.name, &meta.content_type, &blob)
                    .await
                    .is_ok()
            } else {
                false
            };
            if !rewritten && kind == HoldBody::LegacyBytes {
                continue;
            }
        }
        drop(record);
        if legacy {
            // Let the browser drop the old byte array before the next clip is opened.
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
        out.push(meta);
    }
    Ok(out)
}

pub async fn load_blob(project_id: &str, media_id: &str) -> Result<Blob, String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (project_id, media_id);
        return Err("this clip is not in this browser".into());
    }
    #[cfg(target_arch = "wasm32")]
    {
        load_blob_wasm(project_id, media_id).await
    }
}

#[cfg(target_arch = "wasm32")]
async fn load_blob_wasm(project_id: &str, media_id: &str) -> Result<Blob, String> {
    let record = get_record(&hold_key(project_id, media_id)).await?;
    if record.is_null() || record.is_undefined() {
        return Err("this clip is not in this browser".into());
    }
    blob_from_record(&record)
}

async fn open_db() -> Result<web_sys::IdbDatabase, String> {
    let window = web_sys::window().ok_or_else(|| "no browser window".to_string())?;
    let factory = window
        .indexed_db()
        .map_err(js_err)?
        .ok_or_else(|| "this browser has no IndexedDB".to_string())?;
    let open = factory.open_with_u32(DB_NAME, DB_VERSION).map_err(js_err)?;
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let on_upgrade = Closure::once(move |event: web_sys::Event| {
            let Some(req) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::IdbOpenDbRequest>().ok())
            else {
                return;
            };
            let Ok(value) = req.result() else {
                return;
            };
            let Ok(db) = value.dyn_into::<web_sys::IdbDatabase>() else {
                return;
            };
            if !db.object_store_names().contains(STORE) {
                let _ = db.create_object_store(STORE);
            }
        });
        open.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));
        on_upgrade.forget();
        let reject_ok = reject.clone();
        let on_ok = Closure::once(move |event: web_sys::Event| {
            let db = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::IdbRequest>().ok())
                .and_then(|req| req.result().ok())
                .and_then(|value| value.dyn_into::<web_sys::IdbDatabase>().ok());
            match db {
                Some(db) => {
                    let _ = resolve.call1(&JsValue::UNDEFINED, &db);
                }
                None => {
                    let _ = reject_ok.call1(
                        &JsValue::UNDEFINED,
                        &JsValue::from_str("could not open the browser store"),
                    );
                }
            }
        });
        let on_err = Closure::once(move |event: web_sys::Event| {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str(&request_message(&event)),
            );
        });
        open.set_onsuccess(Some(on_ok.as_ref().unchecked_ref()));
        open.set_onerror(Some(on_err.as_ref().unchecked_ref()));
        on_ok.forget();
        on_err.forget();
    });
    let value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(js_err)?;
    value
        .dyn_into::<web_sys::IdbDatabase>()
        .map_err(|_| "browser store did not open".to_string())
}

async fn get_record(key: &str) -> Result<JsValue, String> {
    let db = open_db().await?;
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readonly)
        .map_err(js_err)?;
    let store = tx.object_store(STORE).map_err(js_err)?;
    let req = store.get(&JsValue::from_str(key)).map_err(js_err)?;
    await_request(req).await
}

async fn all_keys() -> Result<Vec<String>, String> {
    let db = open_db().await?;
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readonly)
        .map_err(js_err)?;
    let store = tx.object_store(STORE).map_err(js_err)?;
    let req = store.get_all_keys().map_err(js_err)?;
    let value = await_request(req).await?;
    let Some(array) = value.dyn_ref::<js_sys::Array>() else {
        return Ok(Vec::new());
    };
    Ok(array.iter().filter_map(|key| key.as_string()).collect())
}

fn meta_from_record(record: &JsValue) -> Option<HeldMeta> {
    let id = string_field(record, "id")?;
    let name = string_field(record, "name")?;
    let content_type =
        string_field(record, "content_type").unwrap_or_else(|| "application/octet-stream".into());
    Some(HeldMeta {
        id,
        name,
        content_type,
    })
}

fn classify_record(record: &JsValue) -> HoldBody {
    hold_body(has_blob(record), has_binary_bytes(record))
}

fn has_blob(record: &JsValue) -> bool {
    field(record, "blob").is_some_and(|value| value.is_instance_of::<Blob>())
        || field(record, "bytes").is_some_and(|value| value.is_instance_of::<Blob>())
}

fn has_binary_bytes(record: &JsValue) -> bool {
    field(record, "bytes").is_some_and(|value| {
        value.is_instance_of::<js_sys::Uint8Array>()
            || value.is_instance_of::<js_sys::ArrayBuffer>()
    })
}

fn blob_from_record(record: &JsValue) -> Result<Blob, String> {
    if let Some(blob) = field(record, "blob").and_then(|value| value.dyn_into::<Blob>().ok()) {
        return Ok(blob);
    }
    let Some(bytes) = field(record, "bytes") else {
        return Err("this clip is not in this browser".into());
    };
    if let Ok(blob) = bytes.clone().dyn_into::<Blob>() {
        return Ok(blob);
    }
    let array = if let Ok(array) = bytes.clone().dyn_into::<js_sys::Uint8Array>() {
        array
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        js_sys::Uint8Array::new(&bytes)
    } else {
        return Err("this clip is not in this browser".into());
    };
    let parts = js_sys::Array::of1(&array);
    let opts = BlobPropertyBag::new();
    if let Some(content_type) = string_field(record, "content_type") {
        opts.set_type(&content_type);
    }
    Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js_err)
}

async fn blob_bytes(blob: &Blob) -> Result<Vec<u8>, String> {
    let buf = wasm_bindgen_futures::JsFuture::from(blob.array_buffer())
        .await
        .map_err(js_err)?;
    let array = js_sys::Uint8Array::new(&buf);
    let mut bytes = vec![0_u8; array.length() as usize];
    array.copy_to(&mut bytes);
    Ok(bytes)
}

fn field(value: &JsValue, name: &str) -> Option<JsValue> {
    let field = js_sys::Reflect::get(value, &JsValue::from_str(name)).ok()?;
    if field.is_null() || field.is_undefined() {
        None
    } else {
        Some(field)
    }
}

fn await_request(
    req: web_sys::IdbRequest,
) -> impl std::future::Future<Output = Result<JsValue, String>> {
    let promise = js_sys::Promise::new(&mut |resolve, reject| {
        let on_ok = Closure::once(move |event: web_sys::Event| {
            let value = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::IdbRequest>().ok())
                .and_then(|req| req.result().ok())
                .unwrap_or(JsValue::NULL);
            let _ = resolve.call1(&JsValue::UNDEFINED, &value);
        });
        let on_err = Closure::once(move |event: web_sys::Event| {
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str(&request_message(&event)),
            );
        });
        req.set_onsuccess(Some(on_ok.as_ref().unchecked_ref()));
        req.set_onerror(Some(on_err.as_ref().unchecked_ref()));
        on_ok.forget();
        on_err.forget();
    });
    async move {
        let _keep = req;
        wasm_bindgen_futures::JsFuture::from(promise)
            .await
            .map_err(js_err)
    }
}

fn set_field(record: &js_sys::Object, name: &str, value: &JsValue) -> Result<(), String> {
    js_sys::Reflect::set(record, &JsValue::from_str(name), value)
        .map_err(js_err)
        .and_then(|ok| {
            if ok {
                Ok(())
            } else {
                Err("could not write the browser record".into())
            }
        })
}

fn string_field(value: &JsValue, name: &str) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str(name))
        .ok()
        .and_then(|field| field.as_string())
}

fn request_message(event: &web_sys::Event) -> String {
    event
        .target()
        .and_then(|target| target.dyn_into::<web_sys::IdbRequest>().ok())
        .and_then(|req| req.error().ok().flatten())
        .map(|err| err.message())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "the browser could not store the file".into())
}

fn js_err(err: JsValue) -> String {
    err.as_string()
        .or_else(|| {
            js_sys::Reflect::get(&err, &JsValue::from_str("message"))
                .ok()
                .and_then(|value| value.as_string())
        })
        .unwrap_or_else(|| "the browser could not store the file".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_key_is_the_project_and_the_clip() {
        assert_eq!(hold_key("project", "clip"), "project/clip");
        assert_eq!(project_of_key("project/clip"), Some("project"));
        assert_eq!(project_of_key("clip"), None);
        assert_eq!(project_of_key("/clip"), None);
    }

    #[test]
    fn a_poster_stays_out_of_the_bin() {
        assert_eq!(poster_id("clip"), "clip:poster");
        assert!(is_poster_id("clip:poster"));
        assert!(!is_poster_id("clip"));
        assert!(!is_library_file("clip:poster"));
        assert!(is_library_file("clip"));
        assert!(!is_library_file(""));
        assert_eq!(
            hold_key("project", &poster_id("clip")),
            "project/clip:poster"
        );
    }

    #[test]
    fn a_new_clip_stays_a_blob_and_an_old_byte_array_is_wrapped_once() {
        assert_eq!(hold_body(true, false), HoldBody::Blob);
        assert_eq!(hold_body(true, true), HoldBody::Blob);
        assert_eq!(hold_body(false, true), HoldBody::LegacyBytes);
        assert_eq!(hold_body(false, false), HoldBody::Missing);
    }
}
