pub mod aviation;

use std::collections::HashMap;

use autosurgeon::{Reconcile, reconcile::NoKey};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DocSpace(String);

impl From<String> for DocSpace {
    fn from(space: String) -> Self {
        DocSpace(space)
    }
}

impl From<&'static str> for DocSpace {
    fn from(space: &'static str) -> Self {
        space.to_string().into()
    }
}

impl PartialEq<&'static str> for DocSpace {
    fn eq(&self, other: &&'static str) -> bool {
        self.0 == *other
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DocId(String);

impl From<String> for DocId {
    fn from(id: String) -> Self {
        DocId(id)
    }
}

impl From<&'static str> for DocId {
    fn from(id: &'static str) -> Self {
        id.to_string().into()
    }
}

impl PartialEq<&'static str> for DocId {
    fn eq(&self, other: &&'static str) -> bool {
        self.0 == *other
    }
}

pub struct DocumentResolver {
    deserializers: HashMap<(DocSpace, DocId), DeserializerFn>,
}

type DeserializerFn = fn(&[u8]) -> Result<Document, serde_json::Error>;

impl DocumentResolver {
    pub fn new() -> Self {
        let mut this = Self {
            deserializers: HashMap::new(),
        };

        this.register(
            aviation::DOC_SPACE.into(),
            aviation::flights::DOC_ID.into(),
            Document::deserialize_flights,
        );

        this
    }

    fn register(&mut self, space: DocSpace, id: DocId, deserializer: DeserializerFn) {
        self.deserializers.insert((space, id), deserializer);
    }

    fn get_deserializer(&self, space: &DocSpace, id: &DocId) -> Option<DeserializerFn> {
        self.deserializers
            .get(&(space.clone(), id.clone()))
            .cloned()
    }

    pub fn deserialize(
        &self,
        space: &DocSpace,
        id: &DocId,
        data: &[u8],
    ) -> Result<Document, Error> {
        if let Some(deserializer) = self.get_deserializer(space, id) {
            return Ok(deserializer(data)?);
        }

        Err(Error::UnsupportedDoc {
            space: space.clone(),
            id: id.clone(),
        })
    }
}

impl Default for DocumentResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Document {
    Flights(aviation::flights::Flights),
}

impl Reconcile for Document {
    type Key<'a> = NoKey;

    fn reconcile<R: autosurgeon::Reconciler>(&self, reconciler: R) -> Result<(), R::Error> {
        match self {
            Self::Flights(flights) => flights.reconcile(reconciler),
        }
    }
}

impl Document {
    fn deserialize_flights(data: &[u8]) -> Result<Document, serde_json::Error> {
        let flights: aviation::flights::Flights = serde_json::from_slice(data)?;
        Ok(Document::Flights(flights))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Serde JSON error: {0}")]
    JSON(#[from] serde_json::Error),
    #[error("Unsupported {space:?}: {id:?}")]
    UnsupportedDoc { space: DocSpace, id: DocId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize() {
        let data = r#"[{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}]"#;
        let resolver = DocumentResolver::new();
        let doc = resolver
            .deserialize(
                &aviation::DOC_SPACE.into(),
                &aviation::flights::DOC_ID.into(),
                data.as_bytes(),
            )
            .unwrap();
        let Document::Flights(flights) = doc;
        assert_eq!(flights.iter().collect::<Vec<_>>().len(), 1);
    }

    #[test]
    fn unsupported() {
        let data = r#"[{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}]"#;
        let resolver = DocumentResolver::new();
        assert!(matches!(
            resolver.deserialize(
                &aviation::DOC_SPACE.into(),
                &"dummy".into(),
                data.as_bytes(),
            ),
            Err(Error::UnsupportedDoc { .. })
        ));
    }
}
