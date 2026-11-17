use std::fmt;

use serde::{
    Deserialize, Deserializer,
    de::{Error, MapAccess, Visitor, value::MapAccessDeserializer},
};

pub(crate) struct Record {
    pub(crate) id: i64,
    values: Vec<i64>,
}

impl Record {
    pub(crate) fn score(&self) -> i64 {
        let weighted_sum: i64 = self
            .values
            .iter()
            .enumerate()
            .map(|(index, &value)| (index as i64 + 1) * value)
            .sum();
        self.id + weighted_sum
    }
}

impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RecordVisitor;

        impl<'de> Visitor<'de> for RecordVisitor {
            type Value = Record;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a record object with id and values")
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Record, M::Error> {
                #[derive(Deserialize)]
                struct Fields {
                    id: i64,
                    values: Vec<i64>,
                }

                let Fields { id, values } = Fields::deserialize(MapAccessDeserializer::new(map))?;
                if !(0..=1_000_000).contains(&id)
                    || values.len() > 256
                    || !values
                        .iter()
                        .all(|value| (-1_000_000..=1_000_000).contains(value))
                {
                    return Err(M::Error::custom("record outside score bounds"));
                }
                Ok(Record { id, values })
            }
        }

        // A derived struct deserializer also accepts positional arrays.
        deserializer.deserialize_map(RecordVisitor)
    }
}
