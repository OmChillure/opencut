const KEY: &str = "opencut_user";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn current_email() -> Option<String> {
    storage()?.get_item(KEY).ok().flatten()
}

pub fn is_signed_in() -> bool {
    current_email().is_some()
}

pub fn sign_in(email: &str) {
    if let Some(store) = storage() {
        let _ = store.set_item(KEY, email);
    }
}

pub fn sign_out() {
    if let Some(store) = storage() {
        let _ = store.remove_item(KEY);
    }
}
