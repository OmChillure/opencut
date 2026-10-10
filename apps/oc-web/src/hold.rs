//! Imported pictures stay in this browser. IndexedDB is per site visitor, so the
//! editor works without sending the file to Cloudflare or to the server disk.

use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use web_sys::IdbTransactionMode;

const DB_NAME: &str = "opencut-media";
const STORE: &str = "files";
const DB_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldFile {
    pub id: String,
    pub name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

#[must_use]
pub fn hold_key(project_id: &str, media_id: &str) -> String {
    format!("{project_id}/{media_id}")
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

pub async fn save_file(
    project_id: &str,
    media_id: &str,
    name: &str,
    content_type: &str,
    bytes: &[u8],
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
    let array = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
    array.copy_from(bytes);
    set_field(&record, "bytes", &array)?;
    let req = store
        .put_with_key(&record, &JsValue::from_str(&hold_key(project_id, media_id)))
        .map_err(js_err)?;
    await_request(req).await?;
    Ok(())
}

pub async fn files_for_project(project_id: &str) -> Result<Vec<HeldFile>, String> {
    let db = open_db().await?;
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readonly)
        .map_err(js_err)?;
    let store = tx.object_store(STORE).map_err(js_err)?;
    let req = store.get_all().map_err(js_err)?;
    let value = await_request(req).await?;
    Ok(files_in_project(&value, project_id))
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

fn files_in_project(value: &JsValue, project_id: &str) -> Vec<HeldFile> {
    let Some(array) = value.dyn_ref::<js_sys::Array>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for record in array.iter() {
        if string_field(&record, "project_id").as_deref() != Some(project_id) {
            continue;
        }
        let Some(id) = string_field(&record, "id") else {
            continue;
        };
        let Some(name) = string_field(&record, "name") else {
            continue;
        };
        let content_type = string_field(&record, "content_type")
            .unwrap_or_else(|| "application/octet-stream".into());
        let Some(bytes) = bytes_field(&record, "bytes") else {
            continue;
        };
        out.push(HeldFile {
            id,
            name,
            content_type,
            bytes,
        });
    }
    out
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

fn bytes_field(value: &JsValue, name: &str) -> Option<Vec<u8>> {
    let field = js_sys::Reflect::get(value, &JsValue::from_str(name)).ok()?;
    if field.is_null() || field.is_undefined() {
        return None;
    }
    let array = if let Ok(array) = field.clone().dyn_into::<js_sys::Uint8Array>() {
        array
    } else if field.is_instance_of::<js_sys::ArrayBuffer>() {
        js_sys::Uint8Array::new(&field)
    } else {
        return None;
    };
    let mut bytes = vec![0_u8; array.length() as usize];
    array.copy_to(&mut bytes);
    Some(bytes)
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
}
