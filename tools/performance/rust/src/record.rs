use std::fmt;

use serde::{
    Deserialize, Deserializer,
    de::{Error, MapAccess, Visitor, value::MapAccessDeserializer},
};

pub(crate) struct Record {
    pub(crate) id: i64,
    values: Vec<ScoreInteger>,
}

impl Record {
    pub(crate) fn score(&self) -> i64 {
        let weighted_sum: i64 = self
            .values
            .iter()
            .enumerate()
            .map(|(index, value)| (index as i64 + 1) * value.0)
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
                    id: ScoreInteger,
                    values: Vec<ScoreInteger>,
                }

                let Fields { id, values } = Fields::deserialize(MapAccessDeserializer::new(map))?;
                if id.0 < 0 || values.len() > 256 {
                    return Err(M::Error::custom("record outside score bounds"));
                }
                Ok(Record { id: id.0, values })
            }
        }

        // A derived struct deserializer also accepts positional arrays.
        deserializer.deserialize_map(RecordVisitor)
    }
}

struct ScoreInteger(i64);

impl<'de> Deserialize<'de> for ScoreInteger {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IntegerVisitor;

        impl Visitor<'_> for IntegerVisitor {
            type Value = ScoreInteger;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an integral number between -1000000 and 1000000")
            }

            fn visit_i64<E: Error>(self, value: i64) -> Result<ScoreInteger, E> {
                if (-1_000_000..=1_000_000).contains(&value) {
                    Ok(ScoreInteger(value))
                } else {
                    Err(E::custom("number outside score bounds"))
                }
            }

            fn visit_u64<E: Error>(self, value: u64) -> Result<ScoreInteger, E> {
                let value = i64::try_from(value).map_err(E::custom)?;
                self.visit_i64(value)
            }

            fn visit_f64<E: Error>(self, value: f64) -> Result<ScoreInteger, E> {
                if value.fract() == 0.0 && (-1_000_000.0..=1_000_000.0).contains(&value) {
                    Ok(ScoreInteger(value as i64))
                } else {
                    Err(E::custom("expected a bounded integral JSON number"))
                }
            }
        }

        deserializer.deserialize_any(IntegerVisitor)
    }
}
