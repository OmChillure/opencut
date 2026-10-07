const USER_KEY: &str = "opencut_user";
const TOKEN_KEY: &str = "opencut_token";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

pub fn current_email() -> Option<String> {
    storage()?
        .get_item(USER_KEY)
        .ok()
        .flatten()
        .filter(|email| !email.is_empty())
}

pub fn current_token() -> Option<String> {
    storage()?
        .get_item(TOKEN_KEY)
        .ok()
        .flatten()
        .filter(|token| !token.is_empty())
}

pub fn is_signed_in() -> bool {
    current_email().is_some() && current_token().is_some()
}

pub fn sign_in(email: &str, token: &str) {
    if let Some(store) = storage() {
        let _ = store.set_item(USER_KEY, email);
        let _ = store.set_item(TOKEN_KEY, token);
    }
}

pub fn sign_out() {
    if let Some(store) = storage() {
        let _ = store.remove_item(USER_KEY);
        let _ = store.remove_item(TOKEN_KEY);
    }
}

/// Reject a login form before it hits the server.
pub fn login_form_error(email: &str, password: &str) -> Option<&'static str> {
    let mail = email.trim();
    if mail.is_empty() || !mail.contains('@') {
        return Some("Enter a valid email");
    }
    if password.chars().count() < 4 {
        return Some("Password must be at least 4 characters");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_form_rejects_a_short_password_and_a_bare_name() {
        assert_eq!(login_form_error("", "secret"), Some("Enter a valid email"));
        assert_eq!(
            login_form_error("studio", "secret"),
            Some("Enter a valid email")
        );
        assert_eq!(
            login_form_error("you@studio.com", "ab"),
            Some("Password must be at least 4 characters")
        );
        assert_eq!(login_form_error(" you@studio.com ", "abcd"), None);
    }
}
