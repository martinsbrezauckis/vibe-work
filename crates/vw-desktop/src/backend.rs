//! Typed Linux AT-SPI bus adapter with uncached property reads.
//! Depends on atspi/zbus; called by the window-independent engine.

use atspi::proxy::{
    accessible::AccessibleProxy, action::ActionProxy, component::ComponentProxy,
    editable_text::EditableTextProxy, text::TextProxy,
};
use atspi::zbus::{self, fdo::DBusProxy, names::BusName, proxy::CacheProperties};
use atspi::{AccessibilityConnection, CoordType, Interface, ObjectRefOwned};

use crate::engine::{Backend, Mutation};
use crate::model::{Bounds, Element, ElementId, Error, MAX_TEXT_BYTES, TextValue};

pub(crate) struct Atspi {
    bus: AccessibilityConnection,
}

impl Atspi {
    pub(crate) async fn connect() -> Result<Self, Error> {
        Ok(Self {
            bus: AccessibilityConnection::new().await?,
        })
    }

    pub(crate) async fn connect_at(address: &str) -> Result<Self, Error> {
        Ok(Self {
            bus: AccessibilityConnection::from_address(address.parse()?).await?,
        })
    }

    async fn accessible(&self, id: &ElementId) -> Result<AccessibleProxy<'_>, Error> {
        Ok(AccessibleProxy::builder(self.bus.connection())
            .destination(id.bus.clone())?
            .path(id.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?)
    }

    async fn require(
        &self,
        id: &ElementId,
        interface: Interface,
        label: &'static str,
    ) -> Result<(), Error> {
        if !self
            .accessible(id)
            .await?
            .get_interfaces()
            .await?
            .contains(interface)
        {
            return Err(Error::Unsupported(label));
        }
        Ok(())
    }
}

impl Backend for Atspi {
    async fn root(&self) -> Result<ElementId, Error> {
        let proxy = self.bus.root_accessible_on_registry().await?;
        let registry = BusName::try_from("org.a11y.atspi.Registry").map_err(zbus::Error::from)?;
        let bus = DBusProxy::new(self.bus.connection())
            .await?
            .get_name_owner(registry)
            .await
            .map_err(zbus::Error::from)?;
        Ok(ElementId {
            bus,
            path: proxy.inner().path().to_owned().into(),
        })
    }

    async fn read(&self, id: &ElementId) -> Result<Element, Error> {
        let accessible = self.accessible(id).await?;
        let role = accessible.get_role().await?;
        let name = accessible.name().await?;
        let state = accessible.get_state().await?;
        let bounds = if accessible
            .get_interfaces()
            .await?
            .contains(Interface::Component)
        {
            let component = ComponentProxy::builder(self.bus.connection())
                .destination(id.bus.clone())?
                .path(id.path.clone())?
                .cache_properties(CacheProperties::No)
                .build()
                .await?;
            let (x, y, width, height) = component.get_extents(CoordType::Screen).await?;
            Some(Bounds {
                x,
                y,
                width,
                height,
            })
        } else {
            None
        };
        Ok(Element {
            id: id.clone(),
            role,
            name,
            name_truncated: false,
            state,
            bounds,
            depth: 0,
        })
    }

    async fn child_count(&self, id: &ElementId) -> Result<i32, Error> {
        Ok(self.accessible(id).await?.child_count().await?)
    }

    async fn child(&self, id: &ElementId, index: i32) -> Result<ElementId, Error> {
        let object: ObjectRefOwned = self.accessible(id).await?.get_child_at_index(index).await?;
        let bus = object
            .name()
            .ok_or(Error::MissingElement)?
            .to_owned()
            .into();
        Ok(ElementId {
            bus,
            path: object.path().to_owned().into(),
        })
    }

    async fn text(&self, id: &ElementId) -> Result<TextValue, Error> {
        self.require(id, Interface::Text, "Text").await?;
        let text = TextProxy::builder(self.bus.connection())
            .destination(id.bus.clone())?
            .path(id.path.clone())?
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        let count = text.character_count().await?;
        if count < 0 {
            return Err(Error::InvalidReply("text character count"));
        }
        let limit = i32::try_from(MAX_TEXT_BYTES).map_err(|_| Error::SizeLimit("text"))?;
        Ok(TextValue {
            value: text.get_text(0, count.min(limit)).await?,
            truncated: count > limit,
        })
    }

    async fn mutate(&self, id: &ElementId, mutation: Mutation) -> Result<bool, Error> {
        match mutation {
            Mutation::Press => {
                self.require(id, Interface::Action, "Action").await?;
                let action = ActionProxy::builder(self.bus.connection())
                    .destination(id.bus.clone())?
                    .path(id.path.clone())?
                    .cache_properties(CacheProperties::No)
                    .build()
                    .await?;
                if action.n_actions().await? <= 0 {
                    return Err(Error::Unsupported("default Action"));
                }
                Ok(action.do_action(0).await?)
            }
            Mutation::SetText(value) => {
                self.require(id, Interface::EditableText, "EditableText")
                    .await?;
                let text = EditableTextProxy::builder(self.bus.connection())
                    .destination(id.bus.clone())?
                    .path(id.path.clone())?
                    .cache_properties(CacheProperties::No)
                    .build()
                    .await?;
                Ok(text.set_text_contents(&value).await?)
            }
            Mutation::Focus => {
                self.require(id, Interface::Component, "Component").await?;
                let component = ComponentProxy::builder(self.bus.connection())
                    .destination(id.bus.clone())?
                    .path(id.path.clone())?
                    .cache_properties(CacheProperties::No)
                    .build()
                    .await?;
                Ok(component.grab_focus().await?)
            }
        }
    }
}
