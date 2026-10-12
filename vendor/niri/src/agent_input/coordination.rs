use smithay::reexports::wayland_server::backend::ClientId;

/// Human ownership is tracked by connection, because one client can share its
/// pointer and keyboard state between several windows.
#[derive(Default)]
pub(crate) struct Coordination {
    pub revision: u64,
    pub pointer: Option<ClientId>,
    pub keyboard: Option<ClientId>,
}

impl Coordination {
    pub fn human_owns(&self, client: &ClientId) -> bool {
        self.pointer.as_ref() == Some(client) || self.keyboard.as_ref() == Some(client)
    }

    pub fn pointer_activity(&mut self, client: Option<ClientId>, keys_held: bool) -> bool {
        let keyboard = if keys_held {
            self.keyboard.clone()
        } else {
            None
        };
        self.update(client, keyboard)
    }

    pub fn pointer_focus_changed(&mut self, client: Option<ClientId>) -> bool {
        self.update(client, self.keyboard.clone())
    }

    pub fn keyboard_activity(&mut self, client: Option<ClientId>) -> bool {
        self.update(self.pointer.clone(), client)
    }

    pub fn yield_human(&mut self) -> bool {
        self.update(None, None)
    }

    fn update(&mut self, pointer: Option<ClientId>, keyboard: Option<ClientId>) -> bool {
        if self.pointer == pointer && self.keyboard == keyboard {
            return false;
        }
        self.pointer = pointer;
        self.keyboard = keyboard;
        self.revision += 1;
        true
    }
}
