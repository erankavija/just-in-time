//! The one list envelope, `{"count": N, "<key>": [...], ...extras}`, shared by
//! every list-emitting response. `count` is derived from the collection, never
//! stored, and the collection's key comes from a [`CollectionKey`] marker owned
//! by the response.

use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, ObjectValidation, Schema, SchemaObject};
use schemars::JsonSchema;
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde::ser::{SerializeMap, Serializer};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::marker::PhantomData;

const COUNT_KEY: &str = "count";

/// Names the key a [`ListEnvelope`] carries its collection under.
///
/// The key belongs to the response rather than the item type, since one item
/// type can be listed under several keys.
pub trait CollectionKey {
    /// The collection's serialized key.
    const KEY: &'static str;

    /// The schema name of an envelope owned by one response, which replaces
    /// the generic `ListEnvelope_..` name. Keys shared by several responses
    /// keep `None`.
    const SCHEMA_NAME: Option<&'static str> = None;
}

/// Declares the [`CollectionKey`] marker for each list-envelope collection key.
macro_rules! collection_keys {
    ($($(#[$meta:meta])* $marker:ident = $key:literal $(as $schema:literal)?;)+) => {$(
        $(#[$meta])*
        pub enum $marker {}

        impl $crate::domain::list_envelope::CollectionKey for $marker {
            const KEY: &'static str = $key;
            $(const SCHEMA_NAME: Option<&'static str> = Some($schema);)?
        }
    )+};
}

pub(crate) use collection_keys;

/// A collection under `K::KEY`, its derived `count`, and the flattened fields
/// of `E`, which is a struct or `()`.
///
/// Deserialization rejects a `count` other than the collection's length.
pub struct ListEnvelope<K: CollectionKey, T, E = ()> {
    /// The collection, in the response's documented order.
    pub items: Vec<T>,
    /// Response fields serialized beside `count` and the collection.
    pub extras: E,
    key: PhantomData<fn() -> K>,
}

impl<K: CollectionKey, T> ListEnvelope<K, T> {
    /// Wrap `items` without extras.
    pub fn new(items: Vec<T>) -> Self {
        Self::with_extras(items, ())
    }
}

impl<K: CollectionKey, T, E> ListEnvelope<K, T, E> {
    /// Wrap `items` beside `extras`.
    pub fn with_extras(items: Vec<T>, extras: E) -> Self {
        Self {
            items,
            extras,
            key: PhantomData,
        }
    }

    /// Number of entries in the collection.
    pub fn count(&self) -> usize {
        self.items.len()
    }
}

impl<K: CollectionKey, T: fmt::Debug, E: fmt::Debug> fmt::Debug for ListEnvelope<K, T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListEnvelope")
            .field("key", &K::KEY)
            .field("items", &self.items)
            .field("extras", &self.extras)
            .finish()
    }
}

impl<K: CollectionKey, T: Clone, E: Clone> Clone for ListEnvelope<K, T, E> {
    fn clone(&self) -> Self {
        Self::with_extras(self.items.clone(), self.extras.clone())
    }
}

impl<K: CollectionKey, T: PartialEq, E: PartialEq> PartialEq for ListEnvelope<K, T, E> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items && self.extras == other.extras
    }
}

impl<K: CollectionKey, T: Eq, E: Eq> Eq for ListEnvelope<K, T, E> {}

/// The collection as a one-entry map under `K::KEY`, so serde can flatten it
/// beside `count` and the extras.
struct KeyedItems<K, T> {
    items: T,
    key: PhantomData<fn() -> K>,
}

impl<K: CollectionKey, T: Serialize> Serialize for KeyedItems<K, &[T]> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry(K::KEY, self.items)?;
        map.end()
    }
}

impl<'de, K: CollectionKey, T: Deserialize<'de>> Deserialize<'de> for KeyedItems<K, Vec<T>> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyedVisitor<K, T>(PhantomData<fn() -> (K, T)>);

        impl<'de, K: CollectionKey, T: Deserialize<'de>> Visitor<'de> for KeyedVisitor<K, T> {
            type Value = Vec<T>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a map with a `{}` collection", K::KEY)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Vec<T>, A::Error> {
                let mut items = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == K::KEY {
                        items = Some(map.next_value()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                items.ok_or_else(|| de::Error::missing_field(K::KEY))
            }
        }

        deserializer
            .deserialize_map(KeyedVisitor::<K, T>(PhantomData))
            .map(|items| Self {
                items,
                key: PhantomData,
            })
    }
}

#[derive(Serialize)]
#[serde(bound(serialize = "T: Serialize, E: Serialize"))]
struct WireRef<'a, K: CollectionKey, T, E> {
    count: usize,
    #[serde(flatten)]
    items: KeyedItems<K, &'a [T]>,
    #[serde(flatten)]
    extras: &'a E,
}

#[derive(Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>, E: Deserialize<'de>"))]
struct WireOwned<K: CollectionKey, T, E> {
    count: usize,
    #[serde(flatten)]
    items: KeyedItems<K, Vec<T>>,
    #[serde(flatten)]
    extras: E,
}

impl<K: CollectionKey, T: Serialize, E: Serialize> Serialize for ListEnvelope<K, T, E> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        WireRef {
            count: self.count(),
            items: KeyedItems::<K, _> {
                items: self.items.as_slice(),
                key: PhantomData,
            },
            extras: &self.extras,
        }
        .serialize(serializer)
    }
}

impl<'de, K: CollectionKey, T: Deserialize<'de>, E: Deserialize<'de>> Deserialize<'de>
    for ListEnvelope<K, T, E>
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = WireOwned::<K, T, E>::deserialize(deserializer)?;
        let items = wire.items.items;
        if wire.count != items.len() {
            return Err(de::Error::custom(format_args!(
                "`{COUNT_KEY}` is {} but `{}` holds {} entries",
                wire.count,
                K::KEY,
                items.len()
            )));
        }
        Ok(Self::with_extras(items, wire.extras))
    }
}

impl<K: CollectionKey, T: JsonSchema, E: JsonSchema> JsonSchema for ListEnvelope<K, T, E> {
    fn schema_name() -> String {
        K::SCHEMA_NAME.map_or_else(
            || {
                format!(
                    "ListEnvelope_{}_of_{}_with_{}",
                    K::KEY,
                    T::schema_name(),
                    E::schema_name()
                )
            },
            str::to_owned,
        )
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut object = ObjectValidation::default();
        object
            .properties
            .insert(COUNT_KEY.to_string(), generator.subschema_for::<usize>());
        object
            .properties
            .insert(K::KEY.to_string(), generator.subschema_for::<Vec<T>>());
        object.required.insert(COUNT_KEY.to_string());
        object.required.insert(K::KEY.to_string());
        // `()` has a null schema and contributes no field.
        if let Schema::Object(SchemaObject {
            object: Some(extras),
            ..
        }) = E::json_schema(generator)
        {
            object.properties.extend(extras.properties);
            object.required.extend(extras.required);
        }
        SchemaObject {
            instance_type: Some(InstanceType::Object.into()),
            object: Some(Box::new(object)),
            ..Default::default()
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::{CollectionKey, ListEnvelope};
    use schemars::schema::RootSchema;
    use schemars::{schema_for, JsonSchema};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeSet;

    enum Entries {}

    impl CollectionKey for Entries {
        const KEY: &'static str = "entries";
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
    struct Scope {
        scope: String,
    }

    fn serialized_keys(value: impl Serialize) -> BTreeSet<String> {
        serde_json::to_value(value)
            .expect("an envelope serializes")
            .as_object()
            .expect("an envelope serializes as an object")
            .keys()
            .cloned()
            .collect()
    }

    fn schema_keys(schema: RootSchema) -> (BTreeSet<String>, BTreeSet<String>) {
        let object = schema
            .schema
            .object
            .expect("an envelope schema is an object");
        (object.properties.keys().cloned().collect(), object.required)
    }

    #[test]
    fn test_list_envelope_serializes_items_under_the_collection_key() {
        let envelope = ListEnvelope::<Entries, u8>::new(vec![1, 2, 3]);

        let value = serde_json::to_value(&envelope).expect("an envelope serializes");

        assert_eq!(value[Entries::KEY], serde_json::json!(envelope.items));
    }

    #[test]
    fn test_list_envelope_schema_names_exactly_the_serialized_keys() {
        let plain = ListEnvelope::<Entries, u8>::new(vec![1]);
        let (properties, required) = schema_keys(schema_for!(ListEnvelope<Entries, u8>));
        assert_eq!(properties, serialized_keys(&plain));
        assert_eq!(required, properties);

        let scoped = ListEnvelope::<Entries, u8, Scope>::with_extras(
            vec![1],
            Scope {
                scope: "all".to_string(),
            },
        );
        let (properties, required) = schema_keys(schema_for!(ListEnvelope<Entries, u8, Scope>));
        assert_eq!(properties, serialized_keys(&scoped));
        assert_eq!(required, properties);
    }

    #[test]
    fn test_list_envelope_deserialize_round_trips_items_and_extras() {
        let envelope = ListEnvelope::<Entries, u8, Scope>::with_extras(
            vec![4, 5],
            Scope {
                scope: "open".to_string(),
            },
        );
        let value = serde_json::to_value(&envelope).expect("an envelope serializes");

        let parsed: ListEnvelope<Entries, u8, Scope> =
            serde_json::from_value(value).expect("a serialized envelope deserializes");

        assert_eq!(parsed, envelope);
    }

    #[test]
    fn test_list_envelope_deserialize_rejects_count_other_than_collection_length() {
        let mut value = serde_json::to_value(ListEnvelope::<Entries, u8>::new(vec![1, 2]))
            .expect("an envelope serializes");
        value["count"] = serde_json::json!(3);

        let parsed = serde_json::from_value::<ListEnvelope<Entries, u8>>(value);

        assert!(parsed.is_err());
    }

    #[test]
    fn test_list_envelope_schema_name_key_declared_name_replaces_generic_name() {
        enum Named {}
        impl CollectionKey for Named {
            const KEY: &'static str = "entries";
            const SCHEMA_NAME: Option<&'static str> = Some("NamedResult");
        }

        assert_eq!(ListEnvelope::<Named, u8>::schema_name(), "NamedResult");
        assert_eq!(
            serialized_keys(ListEnvelope::<Named, u8>::new(vec![1])),
            serialized_keys(ListEnvelope::<Entries, u8>::new(vec![1]))
        );
    }

    #[test]
    fn test_list_envelope_schema_name_differs_per_collection_key() {
        enum Others {}
        impl CollectionKey for Others {
            const KEY: &'static str = "others";
        }

        assert_ne!(
            ListEnvelope::<Entries, u8>::schema_name(),
            ListEnvelope::<Others, u8>::schema_name()
        );
    }
}
