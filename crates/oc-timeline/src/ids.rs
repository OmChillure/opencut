use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_ty {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            #[must_use]
            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_ty!(ProjectId);
id_ty!(TrackId);
id_ty!(ClipId);
id_ty!(MediaId);
id_ty!(GroupId);
id_ty!(LinkId);
id_ty!(MarkerId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_roundtrip_keeps_the_same_value() {
        let raw = Uuid::nil();
        let id = ClipId::from_uuid(raw);
        assert_eq!(id.as_uuid(), raw);
        assert_eq!(id.to_string(), raw.to_string());
        let back: ClipId = serde_json::from_str(&serde_json::to_string(&id).unwrap()).unwrap();
        assert_eq!(back, id);
    }
}
