use std::fmt;

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, Visitor, value::MapAccessDeserializer},
};

pub(crate) struct Record {
    pub(crate) id: u64,
    values: Vec<i64>,
}

impl Record {
    pub(crate) fn score(&self) -> i128 {
        let weighted_sum: i128 = self
            .values
            .iter()
            .enumerate()
            .map(|(index, &value)| (index as i128 + 1) * i128::from(value))
            .sum();
        i128::from(self.id) + weighted_sum
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
                #[serde(deny_unknown_fields)]
                struct Fields {
                    id: u64,
                    values: Vec<i64>,
                }

                let Fields { id, values } = Fields::deserialize(MapAccessDeserializer::new(map))?;
                Ok(Record { id, values })
            }
        }

        // A derived struct deserializer also accepts positional arrays.
        deserializer.deserialize_map(RecordVisitor)
    }
}
