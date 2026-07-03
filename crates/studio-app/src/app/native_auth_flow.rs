use super::native_auth::{NativeAuthStore, default_native_auth_store_path, read_native_auth_store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NativeAuthStep {
    Login,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NativeAuthStartup {
    Authenticated,
    Flow(NativeAuthFlowState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeAuthFlowState {
    pub(super) step: NativeAuthStep,
    pub(super) identifier: String,
    pub(super) code: String,
    pub(super) pending_sign_in_id: Option<String>,
    pub(super) pending_phone_number_id: Option<String>,
    pub(super) is_pending: bool,
    pub(super) error: Option<String>,
}

impl NativeAuthFlowState {
    pub(super) fn startup_from_default_store() -> NativeAuthStartup {
        let store_path = default_native_auth_store_path();
        match read_native_auth_store(&store_path) {
            Ok(store) => Self::startup_from_store(Some(&store)),
            Err(_) => NativeAuthStartup::Flow(Self::login_prompt()),
        }
    }

    pub(super) fn startup_from_store(store: Option<&NativeAuthStore>) -> NativeAuthStartup {
        let Some(store) = store else {
            return NativeAuthStartup::Flow(Self::login_prompt());
        };
        if let Some(flow) = Self::pending_code_from_store(store) {
            return NativeAuthStartup::Flow(flow);
        }
        if store.cookies.is_empty() {
            NativeAuthStartup::Flow(Self::login_prompt())
        } else {
            NativeAuthStartup::Authenticated
        }
    }

    fn pending_code_from_store(store: &NativeAuthStore) -> Option<Self> {
        let pending_sign_in_id = store
            .pending_sign_in_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let pending_phone_number_id = store
            .pending_phone_number_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        Some(Self::pending_code(
            pending_sign_in_id,
            pending_phone_number_id,
        ))
    }

    pub(super) fn login_prompt() -> Self {
        Self {
            step: NativeAuthStep::Login,
            identifier: String::new(),
            code: String::new(),
            pending_sign_in_id: None,
            pending_phone_number_id: None,
            is_pending: false,
            error: None,
        }
    }

    pub(super) fn pending_code(
        pending_sign_in_id: String,
        pending_phone_number_id: Option<String>,
    ) -> Self {
        Self {
            step: NativeAuthStep::Code,
            identifier: String::new(),
            code: String::new(),
            pending_sign_in_id: Some(pending_sign_in_id),
            pending_phone_number_id,
            is_pending: false,
            error: None,
        }
    }

    pub(super) fn is_code_step(&self) -> bool {
        self.step == NativeAuthStep::Code
    }

    pub(super) fn can_submit_code(&self) -> bool {
        self.is_code_step() && !self.is_pending && !self.code.trim().is_empty()
    }

    pub(super) fn can_start_login(&self) -> bool {
        self.step == NativeAuthStep::Login && !self.is_pending && !self.identifier.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_auth::{NativeAuthCookie, NativeAuthStore};
    use super::*;

    #[test]
    fn missing_store_launches_login_flow() {
        assert_eq!(
            NativeAuthFlowState::startup_from_store(None),
            NativeAuthStartup::Flow(NativeAuthFlowState::login_prompt())
        );
    }

    #[test]
    fn pending_store_launches_phone_code_flow() {
        let store = NativeAuthStore {
            pending_sign_in_id: Some("si_native".to_string()),
            pending_phone_number_id: Some("phone_native".to_string()),
            ..NativeAuthStore::default()
        };

        let NativeAuthStartup::Flow(flow) = NativeAuthFlowState::startup_from_store(Some(&store))
        else {
            panic!("pending store should launch auth flow");
        };

        assert_eq!(flow.step, NativeAuthStep::Code);
        assert_eq!(flow.pending_sign_in_id.as_deref(), Some("si_native"));
        assert_eq!(
            flow.pending_phone_number_id.as_deref(),
            Some("phone_native")
        );
        assert!(!flow.can_submit_code());
    }

    #[test]
    fn cookie_store_launches_authenticated_dashboard() {
        let store = NativeAuthStore {
            cookies: vec![NativeAuthCookie {
                name: "__client".to_string(),
                value: "cookie".to_string(),
                domain: "clerk.babel.audio".to_string(),
                path: "/".to_string(),
                secure: true,
                http_only: false,
                same_site: "Lax".to_string(),
                expires_at: None,
            }],
            ..NativeAuthStore::default()
        };

        assert_eq!(
            NativeAuthFlowState::startup_from_store(Some(&store)),
            NativeAuthStartup::Authenticated
        );
    }

    #[test]
    fn phone_code_flow_requires_non_empty_code_and_not_pending() {
        let mut flow = NativeAuthFlowState::pending_code(
            "si_native".to_string(),
            Some("phone_native".to_string()),
        );

        flow.code = " 123456 ".to_string();
        assert!(flow.can_submit_code());

        flow.is_pending = true;
        assert!(!flow.can_submit_code());

        flow.is_pending = false;
        flow.code = "   ".to_string();
        assert!(!flow.can_submit_code());
    }

    #[test]
    fn login_flow_requires_identifier_and_not_pending() {
        let mut flow = NativeAuthFlowState::login_prompt();

        assert!(!flow.can_start_login());

        flow.identifier = " user@example.com ".to_string();
        assert!(flow.can_start_login());

        flow.is_pending = true;
        assert!(!flow.can_start_login());
    }
}
