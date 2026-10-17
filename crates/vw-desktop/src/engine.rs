//! Accessibility tree selection and action postconditions independent of a window system.
//! Depends on the backend contract and model; called by Desktop and deterministic fixture tests.

use atspi::{Role, State};

use crate::model::{
    Action, Element, ElementId, Error, MAX_DEPTH, MAX_ELEMENTS, MAX_ID_BYTES, MAX_NAME_BYTES,
    MAX_TEXT_BYTES, Postcondition, Receipt, Snapshot, TextValue, Verification, cap,
};

pub(crate) enum Mutation {
    Press,
    SetText(String),
    Focus,
}

pub(crate) trait Backend {
    async fn root(&self) -> Result<ElementId, Error>;
    async fn read(&self, id: &ElementId) -> Result<Element, Error>;
    async fn child_count(&self, id: &ElementId) -> Result<i32, Error>;
    async fn child(&self, id: &ElementId, index: i32) -> Result<ElementId, Error>;
    async fn text(&self, id: &ElementId) -> Result<TextValue, Error>;
    async fn mutate(&self, id: &ElementId, mutation: Mutation) -> Result<bool, Error>;
}

pub(crate) struct Engine<B>(pub(crate) B);

struct Cursor {
    id: ElementId,
    depth: u16,
    next: i32,
    count: i32,
}

impl<B: Backend> Engine<B> {
    pub(crate) async fn snapshot(&self, window: Option<&str>) -> Result<Snapshot, Error> {
        let id = self.select_window(window).await?;
        let mut snapshot = Snapshot {
            window: id.clone(),
            elements: vec![self.read(&id).await?],
            truncated: false,
        };
        let mut stack = vec![self.cursor(id, 0).await?];
        while let Some(cursor) = stack.last_mut() {
            if cursor.next >= cursor.count {
                stack.pop();
                continue;
            }
            if snapshot.elements.len() == MAX_ELEMENTS {
                snapshot.truncated = true;
                break;
            }
            if cursor.depth == MAX_DEPTH {
                snapshot.truncated = true;
                stack.pop();
                continue;
            }
            let child = self.0.child(&cursor.id, cursor.next).await?;
            cursor.next += 1;
            let depth = cursor.depth + 1;
            let mut element = self.read(&child).await?;
            element.depth = depth;
            snapshot.elements.push(element);
            stack.push(self.cursor(child, depth).await?);
        }
        Ok(snapshot)
    }

    async fn select_window(&self, name: Option<&str>) -> Result<ElementId, Error> {
        if name.is_some_and(|name| name.len() > MAX_NAME_BYTES) {
            return Err(Error::SizeLimit("window name"));
        }
        let root = self.0.root().await?;
        let applications = self.count(&root).await?;
        let mut inspected = 0;
        let mut selected = None;
        // AT-SPI's desktop root contains applications, whose immediate children are windows.
        for app_index in 0..applications {
            inspected += 1;
            if inspected > MAX_ELEMENTS {
                return Err(Error::DiscoveryLimit);
            }
            let app = self.0.child(&root, app_index).await?;
            for window_index in 0..self.count(&app).await? {
                inspected += 1;
                if inspected > MAX_ELEMENTS {
                    return Err(Error::DiscoveryLimit);
                }
                let id = self.0.child(&app, window_index).await?;
                let element = self.read(&id).await?;
                if !matches!(element.role, Role::Frame | Role::Dialog | Role::Window) {
                    continue;
                }
                let matches = name.map_or_else(
                    || {
                        element.state.contains(State::Active)
                            || element.state.contains(State::Focused)
                    },
                    |name| !element.name_truncated && element.name == name,
                );
                if matches && selected.replace(id).is_some() {
                    return Err(Error::AmbiguousWindow);
                }
            }
        }
        selected.ok_or(Error::WindowNotFound)
    }

    async fn read(&self, id: &ElementId) -> Result<Element, Error> {
        if id.bus.as_str().len() + id.path.as_str().len() > MAX_ID_BYTES {
            return Err(Error::SizeLimit("element id"));
        }
        let mut element = self.0.read(id).await?;
        element.name_truncated |= cap(&mut element.name, MAX_NAME_BYTES);
        Ok(element)
    }

    async fn count(&self, id: &ElementId) -> Result<i32, Error> {
        let count = self.0.child_count(id).await?;
        if count < 0 {
            return Err(Error::InvalidReply("child count"));
        }
        Ok(count)
    }

    async fn cursor(&self, id: ElementId, depth: u16) -> Result<Cursor, Error> {
        let count = self.count(&id).await?;
        Ok(Cursor {
            id,
            depth,
            next: 0,
            count,
        })
    }

    pub(crate) async fn act(&self, id: &ElementId, action: Action) -> Result<Receipt, Error> {
        let (mutation, expected) = match action {
            Action::Press { expected } => (Mutation::Press, expected),
            Action::SetText(value) => {
                (Mutation::SetText(value.clone()), Postcondition::Text(value))
            }
            Action::Focus => (
                Mutation::Focus,
                Postcondition::State {
                    state: State::Focused,
                    present: true,
                },
            ),
        };
        validate(&expected)?;
        let before = self.read(id).await?;
        let actionable = before.state.contains(State::Sensitive)
            && !before.state.contains(State::Defunct)
            && match &mutation {
                Mutation::Press => true,
                Mutation::SetText(_) => before.state.contains(State::Editable),
                Mutation::Focus => before.state.contains(State::Focusable),
            };
        if !actionable {
            return Err(Error::NotActionable);
        }
        let accepted = self.0.mutate(id, mutation).await?;
        Ok(Receipt {
            accepted,
            verification: self.verify(id, &expected).await?,
        })
    }

    pub(crate) async fn verify(
        &self,
        id: &ElementId,
        expected: &Postcondition,
    ) -> Result<Verification, Error> {
        validate(expected)?;
        let observed = self.read(id).await?;
        let mut text = None;
        let matched = match expected {
            Postcondition::Name(name) => !observed.name_truncated && observed.name == *name,
            Postcondition::State { state, present } => observed.state.contains(*state) == *present,
            Postcondition::Text(value) => {
                let mut actual = self.0.text(id).await?;
                actual.truncated |= cap(&mut actual.value, MAX_TEXT_BYTES);
                let matches = !actual.truncated && actual.value == *value;
                text = Some(actual);
                matches
            }
        };
        // A defunct object is never evidence that the desired live control state holds.
        Ok(Verification {
            matched: matched && !observed.state.contains(State::Defunct),
            observed,
            text,
        })
    }
}

fn validate(expected: &Postcondition) -> Result<(), Error> {
    match expected {
        Postcondition::Name(name) if name.len() > MAX_NAME_BYTES => {
            Err(Error::SizeLimit("expected name"))
        }
        Postcondition::Text(text) if text.len() > MAX_TEXT_BYTES => Err(Error::SizeLimit("text")),
        _ => Ok(()),
    }
}
