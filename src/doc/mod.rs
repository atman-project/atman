pub mod aviation;

use autosurgeon::Reconcile;
use aviation::flight::Flight;
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

pub trait Resolver {
    type Error;

    fn deserialize(
        space: &DocSpace,
        id: &DocId,
        data: &[u8],
    ) -> Result<impl Reconcile + Serialize, Self::Error>;
}

pub struct DocResolver;

impl Resolver for DocResolver {
    type Error = Error;

    fn deserialize(
        space: &DocSpace,
        id: &DocId,
        data: &[u8],
    ) -> Result<impl Reconcile + Serialize, Self::Error> {
        if space == &"aviation" && id == &"flight" {
            return Ok(serde_json::from_slice::<Flight>(data)?);
        }
        Err(Error::UnsupportedDoc {
            space: space.clone(),
            id: id.clone(),
        })
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
        let data = r#"{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}"#;
        let space = "aviation".into();
        let id = "flight".into();
        let flight = DocResolver::deserialize(&space, &id, data.as_bytes()).unwrap();
        println!("{}", serde_json::to_string(&flight).unwrap());
    }

    #[test]
    fn unsupported() {
        let data = r#"{"departureLocalTime":"2025-08-03T05:45:10Z","arrivalLocalTime":"2025-08-03T05:45:10Z","departureAirport":"ZZU","arrivalAirport":"ADS","airline":"","aircraft":"","id":"074BD346-C9D9-4BBF-AB95-BDB3AECB1FE0","flightNumber":"","bookingReference":""}"#;
        let space = "aviation".into();
        let id = "dummy".into();
        assert!(matches!(
            DocResolver::deserialize(&space, &id, data.as_bytes()),
            Err(Error::UnsupportedDoc { .. })
        ));
    }
}
