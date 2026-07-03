#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct OutsideInteractionContext {
    pub(super) inside_surface: bool,
    pub(super) has_listbox_ancestor: bool,
    pub(super) has_radix_portal_ancestor: bool,
    pub(super) has_radix_select_content_ancestor: bool,
}

impl OutsideInteractionContext {
    pub(super) const fn plain_outside() -> Self {
        Self {
            inside_surface: false,
            has_listbox_ancestor: false,
            has_radix_portal_ancestor: false,
            has_radix_select_content_ancestor: false,
        }
    }

    #[cfg(test)]
    pub(super) const fn inside_surface() -> Self {
        Self {
            inside_surface: true,
            ..Self::plain_outside()
        }
    }

    #[cfg(test)]
    pub(super) const fn listbox() -> Self {
        Self {
            has_listbox_ancestor: true,
            ..Self::plain_outside()
        }
    }

    #[cfg(test)]
    pub(super) const fn radix_portal() -> Self {
        Self {
            has_radix_portal_ancestor: true,
            ..Self::plain_outside()
        }
    }

    #[cfg(test)]
    pub(super) const fn radix_select_content() -> Self {
        Self {
            has_radix_select_content_ancestor: true,
            ..Self::plain_outside()
        }
    }

    pub(super) fn has_recovered_select_or_portal_target(self) -> bool {
        self.has_listbox_ancestor
            || self.has_radix_portal_ancestor
            || self.has_radix_select_content_ancestor
    }
}
