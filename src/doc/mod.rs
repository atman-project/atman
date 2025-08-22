pub mod aviation;

use std::collections::HashMap;

use autosurgeon::{Reconcile, reconcile::NoKey};
use serde::{Deserialize, Serialize};
use syncman::Syncman;

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

pub struct DocumentResolver<S: Syncman> {
    deserializers: HashMap<(DocSpace, DocId), DeserializerFn>,
    hydraters: HashMap<(DocSpace, DocId), HydrateFn<S>>,
}

type DeserializerFn = fn(&[u8]) -> Result<Document, serde_json::Error>;
type HydrateFn<S> = fn(&S) -> Result<Document, Error>;

impl<S: Syncman> DocumentResolver<S> {
    pub fn new() -> Self {
        let mut this = Self {
            deserializers: HashMap::new(),
            hydraters: HashMap::new(),
        };

        this.register(
            aviation::DOC_SPACE.into(),
            aviation::flights::DOC_ID.into(),
            Document::deserialize_flights,
            Document::hydrate_flights,
        );
        this.register(
            aviation::DOC_SPACE.into(),
            aviation::flight::DOC_ID.into(),
            Document::deserialize_flight,
            Document::hydrate_flight,
        );

        this
    }

    fn register(
        &mut self,
        space: DocSpace,
        id: DocId,
        deserializer: DeserializerFn,
        hydrater: HydrateFn<S>,
    ) {
        self.deserializers
            .insert((space.clone(), id.clone()), deserializer);
        self.hydraters.insert((space, id), hydrater);
    }

    fn get_deserializer(&self, space: &DocSpace, id: &DocId) -> Option<DeserializerFn> {
        self.deserializers
            .get(&(space.clone(), id.clone()))
            .cloned()
    }

    fn get_hydrater(&self, space: &DocSpace, id: &DocId) -> Option<HydrateFn<S>> {
        self.hydraters.get(&(space.clone(), id.clone())).cloned()
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

    pub fn hydrate(&self, space: &DocSpace, id: &DocId, syncman: &S) -> Result<Document, Error> {
        if let Some(hydrater) = self.get_hydrater(space, id) {
            return hydrater(syncman);
        }

        Err(Error::UnsupportedDoc {
            space: space.clone(),
            id: id.clone(),
        })
    }
}

impl<S: Syncman> Default for DocumentResolver<S> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Document {
    Flights(aviation::flights::Flights),
    Flight(aviation::flight::Flight),
}

impl Reconcile for Document {
    type Key<'a> = NoKey;

    fn reconcile<R: autosurgeon::Reconciler>(&self, reconciler: R) -> Result<(), R::Error> {
        match self {
            Self::Flights(flights) => flights.reconcile(reconciler),
            Self::Flight(flight) => flight.reconcile(reconciler),
        }
    }
}

impl Document {
    pub fn serialize(&self) -> Result<Vec<u8>, serde_json::Error> {
        match self {
            Self::Flights(flights) => serde_json::to_vec(flights),
            Self::Flight(flight) => serde_json::to_vec(flight),
        }
    }

    fn deserialize_flights(data: &[u8]) -> Result<Self, serde_json::Error> {
        let flights: aviation::flights::Flights = serde_json::from_slice(data)?;
        Ok(Self::Flights(flights))
    }

    fn hydrate_flights<S: Syncman>(syncman: &S) -> Result<Self, Error> {
        Ok(Document::Flights(
            syncman.get::<aviation::flights::Flights>(),
        ))
    }

    fn deserialize_flight(data: &[u8]) -> Result<Self, serde_json::Error> {
        let flight: aviation::flight::Flight = serde_json::from_slice(data)?;
        Ok(Self::Flight(flight))
    }

    fn hydrate_flight<S: Syncman>(syncman: &S) -> Result<Self, Error> {
        Ok(Document::Flight(syncman.get::<aviation::flight::Flight>()))
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
    use syncman::SyncHandle;

    use super::*;

    #[test]
    fn deserialize_flights() {
        let data = r#"{"flights":[{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}]}"#;
        let resolver = DocumentResolver::<MockSyncman>::new();
        let doc = resolver
            .deserialize(
                &aviation::DOC_SPACE.into(),
                &aviation::flights::DOC_ID.into(),
                data.as_bytes(),
            )
            .unwrap();
        let Document::Flights(flights) = doc else {
            panic!("Expected Flights document");
        };
        assert_eq!(flights.flights().collect::<Vec<_>>().len(), 1);
    }

    #[test]
    fn deserialize_flight() {
        let data = r#"{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}"#;
        let resolver = DocumentResolver::<MockSyncman>::new();
        let doc = resolver
            .deserialize(
                &aviation::DOC_SPACE.into(),
                &aviation::flight::DOC_ID.into(),
                data.as_bytes(),
            )
            .unwrap();
        assert!(matches!(doc, Document::Flight(_)));
    }

    #[test]
    fn unsupported() {
        let data = r#"[{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}]"#;
        let resolver = DocumentResolver::<MockSyncman>::new();
        assert!(matches!(
            resolver.deserialize(
                &aviation::DOC_SPACE.into(),
                &"dummy".into(),
                data.as_bytes(),
            ),
            Err(Error::UnsupportedDoc { .. })
        ));
    }

    struct MockSyncman;

    impl Syncman for MockSyncman {
        type Handle = MockSyncHandle;
        type ObjectId = usize;
        type Property = String;

        fn update<Model>(&mut self, _: &Model)
        where
            Model: Reconcile,
        {
            unimplemented!()
        }

        fn root(&self) -> Self::ObjectId {
            unimplemented!()
        }

        fn update_prop<Obj, Prop, Model>(&mut self, _: Obj, _: Prop, _: &Model)
        where
            Obj: AsRef<Self::ObjectId>,
            Prop: Into<Self::Property>,
            Model: Reconcile,
        {
            unimplemented!()
        }

        fn insert<Model>(&mut self, _: Self::ObjectId, _: usize, _: &Model)
        where
            Model: Reconcile,
        {
            unimplemented!()
        }

        fn get<Model>(&self) -> Model
        where
            Model: autosurgeon::Hydrate,
        {
            unimplemented!()
        }

        fn get_prop<Obj, Prop, Model>(&self, _: Obj, _: Prop) -> Model
        where
            Obj: AsRef<Self::ObjectId>,
            Prop: Into<Self::Property>,
            Model: autosurgeon::Hydrate,
        {
            unimplemented!()
        }

        fn get_object_id<Obj, Prop>(&self, _: Obj, _: Prop) -> Self::ObjectId
        where
            Obj: AsRef<Self::ObjectId>,
            Prop: Into<Self::Property>,
        {
            unimplemented!()
        }

        fn initiate_sync(&mut self) -> Self::Handle {
            unimplemented!()
        }

        fn apply_sync(&mut self, _: &mut Self::Handle, _: &[u8]) {
            unimplemented!()
        }

        fn dump(&self) -> HashMap<String, String> {
            unimplemented!()
        }
    }

    struct MockSyncHandle;

    impl SyncHandle for MockSyncHandle {
        fn generate_message(&mut self) -> Option<Vec<u8>> {
            unimplemented!()
        }
    }
}
