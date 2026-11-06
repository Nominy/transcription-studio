//! Startup auth gate and login-screen state (Babel phone-code or sub-account code).

use std::path::Path;

use super::store::{AuthStore, read_store, trimmed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStep {
    Login,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    Babel,
    Subaccount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthStartup {
    /// The store has Clerk cookies: open the dashboard.
    Authenticated,
    /// Show the login screen in this state.
    Flow(AuthFlow),
}

/// Login-screen state. While a flow is shown, Babel service calls must not be dispatched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFlow {
    pub mode: AuthMode,
    pub step: AuthStep,
    pub identifier: String,
    pub code: String,
    pub subaccount_host_url: String,
    pub subaccount_code: String,
    pub pending_sign_in_id: Option<String>,
    pub pending_phone_number_id: Option<String>,
    pub is_pending: bool,
    pub error: Option<String>,
}

impl AuthFlow {
    /// Startup decision from the store at `store_path`; unreadable/missing store → login prompt.
    pub fn startup(store_path: &Path) -> AuthStartup {
        match read_store(store_path) {
            Ok(store) => Self::startup_from_store(Some(&store)),
            Err(_) => AuthStartup::Flow(Self::login_prompt()),
        }
    }

    /// Pending sign-in → code step; cookies → authenticated; otherwise login prompt.
    pub fn startup_from_store(store: Option<&AuthStore>) -> AuthStartup {
        let Some(store) = store else {
            return AuthStartup::Flow(Self::login_prompt());
        };
        if let Some(sign_in_id) = trimmed(store.pending_sign_in_id.as_deref()) {
            return AuthStartup::Flow(Self::pending_code(
                sign_in_id,
                trimmed(store.pending_phone_number_id.as_deref()),
            ));
        }
        if store.cookies.is_empty() {
            AuthStartup::Flow(Self::login_prompt())
        } else {
            AuthStartup::Authenticated
        }
    }

    fn new(mode: AuthMode, step: AuthStep) -> Self {
        Self {
            mode,
            step,
            identifier: String::new(),
            code: String::new(),
            subaccount_host_url: crate::subaccounts::default_relay_host_url(),
            subaccount_code: String::new(),
            pending_sign_in_id: None,
            pending_phone_number_id: None,
            is_pending: false,
            error: None,
        }
    }

    pub fn login_prompt() -> Self {
        Self::new(AuthMode::Babel, AuthStep::Login)
    }

    /// Code-entry step after [`super::start_login`] succeeded.
    pub fn pending_code(sign_in_id: String, phone_number_id: Option<String>) -> Self {
        Self {
            pending_sign_in_id: Some(sign_in_id),
            pending_phone_number_id: phone_number_id,
            ..Self::new(AuthMode::Babel, AuthStep::Code)
        }
    }

    pub fn subaccount_login_prompt() -> Self {
        Self::new(AuthMode::Subaccount, AuthStep::Login)
    }

    pub fn is_babel_mode(&self) -> bool {
        self.mode == AuthMode::Babel
    }

    pub fn is_subaccount_mode(&self) -> bool {
        self.mode == AuthMode::Subaccount
    }

    pub fn is_code_step(&self) -> bool {
        self.is_babel_mode() && self.step == AuthStep::Code
    }

    pub fn can_submit_code(&self) -> bool {
        self.is_code_step() && !self.is_pending && !self.code.trim().is_empty()
    }

    pub fn can_start_login(&self) -> bool {
        self.is_babel_mode()
            && self.step == AuthStep::Login
            && !self.is_pending
            && !self.identifier.trim().is_empty()
    }

    pub fn can_submit_subaccount_login(&self) -> bool {
        self.is_subaccount_mode()
            && !self.is_pending
            && !self.subaccount_host_url.trim().is_empty()
            && !self.subaccount_code.trim().is_empty()
    }
}
