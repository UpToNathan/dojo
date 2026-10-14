use crate::{Error, Evaluation, MAX_INPUT_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Metric {
    IncludeExclude {
        #[serde(default)]
        include: Vec<String>,
        #[serde(default)]
        exclude: Vec<String>,
    },
    LiteralString {
        expected: String,
        #[serde(default)]
        ignore_case: bool,
    },
    LiteralList {
        expected: Vec<String>,
        #[serde(default)]
        ignore_case: bool,
    },
    Membership {
        expected: Value,
    },
    JsonSubset {
        expected: Map<String, Value>,
    },
    JsonInclusion {
        expected: Map<String, Value>,
    },
    JsonRules {
        #[serde(default)]
        expect: Vec<JsonRule>,
        #[serde(default)]
        unexpect: Vec<JsonRule>,
    },
    FuzzyText {
        expected: String,
    },
    TextEqual {
        expected: String,
        #[serde(default)]
        ignore_whitespace: bool,
    },
    Records {
        #[serde(default)]
        expect: Vec<Map<String, Value>>,
        #[serde(default)]
        unexpect: Vec<Map<String, Value>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathKey {
    Field(String),
    Index(usize),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JsonRule {
    pub key: Vec<PathKey>,
    pub comparison: Comparison,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Comparison {
    Eq {
        reference: Value,
    },
    Ne {
        reference: Value,
    },
    Lt {
        reference: Value,
    },
    Le {
        reference: Value,
    },
    Gt {
        reference: Value,
    },
    Ge {
        reference: Value,
    },
    Approx {
        reference: f64,
        threshold: f64,
    },
    Range {
        minimum: f64,
        maximum: f64,
        minimum_inclusive: bool,
        maximum_inclusive: bool,
    },
    StrListEq {
        reference: Vec<String>,
    },
    StrSetEq {
        reference: Vec<String>,
    },
}

fn invalid() -> Error {
    Error::Invalid("general evaluator evidence or rule type")
}
fn text(value: &Value) -> Result<&str, Error> {
    value.as_str().ok_or_else(invalid)
}
fn strings(value: &Value) -> Result<Vec<&str>, Error> {
    value
        .as_array()
        .ok_or_else(invalid)?
        .iter()
        .map(text)
        .collect()
}
fn numeric(value: &Value) -> Result<f64, Error> {
    let number = value
        .as_f64()
        .or_else(|| value.as_bool().map(|value| f64::from(u8::from(value))))
        .or_else(|| value.as_str().and_then(|text| text.parse::<f64>().ok()))
        .ok_or_else(invalid)?;
    if number.is_finite() {
        Ok(number)
    } else {
        Err(invalid())
    }
}
fn value_at<'a>(value: &'a Value, keys: &[PathKey]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |value, key| match key {
        PathKey::Field(key) => value.as_object()?.get(key),
        PathKey::Index(index) => value.as_array()?.get(*index),
    })
}

fn equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
                return left == right;
            }
            if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
                return left == right;
            }
            let integer_float = |integer: &serde_json::Number, float: f64| {
                if float.trunc() != float {
                    return false;
                }
                if let Some(integer) = integer.as_i64() {
                    float >= i64::MIN as f64
                        && float < 9_223_372_036_854_775_808.0
                        && float as i64 == integer
                } else if let Some(integer) = integer.as_u64() {
                    float >= 0.0 && float < 18_446_744_073_709_551_616.0 && float as u64 == integer
                } else {
                    false
                }
            };
            if left.is_i64() || left.is_u64() {
                integer_float(left, right.as_f64().unwrap())
            } else if right.is_i64() || right.is_u64() {
                integer_float(right, left.as_f64().unwrap())
            } else {
                left.as_f64() == right.as_f64()
            }
        }
        (Value::Bool(value), Value::Number(_)) => equal(&Value::from(u64::from(*value)), right),
        (Value::Number(_), Value::Bool(value)) => equal(left, &Value::from(u64::from(*value))),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .all(|(key, left)| right.get(key).is_some_and(|right| equal(left, right)))
        }
        _ => left == right,
    }
}

fn compare(value: &Value, comparison: &Comparison) -> Result<bool, Error> {
    match comparison {
        Comparison::Eq { reference } => Ok(equal(value, reference)),
        Comparison::Ne { reference } => Ok(!equal(value, reference)),
        Comparison::Lt { reference }
        | Comparison::Le { reference }
        | Comparison::Gt { reference }
        | Comparison::Ge { reference } => {
            let ordering =
                if let (Some(left), Some(right)) = (value.as_str(), reference.as_str()) {
                    left.partial_cmp(right)
                } else if value.is_number() && reference.is_number() {
                    // JSON integer order must not collapse distinct IDs above 2^53.
                    match (
                        value.as_i64(),
                        reference.as_i64(),
                        value.as_u64(),
                        reference.as_u64(),
                    ) {
                        (Some(left), Some(right), _, _) => left.partial_cmp(&right),
                        (_, _, Some(left), Some(right)) => left.partial_cmp(&right),
                        (Some(left), _, _, Some(_)) if left < 0 => Some(std::cmp::Ordering::Less),
                        (_, Some(right), Some(_), _) if right < 0 => {
                            Some(std::cmp::Ordering::Greater)
                        }
                        _ => {
                            if [value, reference].iter().any(|number| {
                                number
                                    .as_u64()
                                    .is_some_and(|number| number > 9_007_199_254_740_992)
                                    || number.as_i64().is_some_and(|number| {
                                        number.unsigned_abs() > 9_007_199_254_740_992
                                    })
                            }) {
                                return Err(Error::Invalid(
                                    "mixed numeric comparison exceeds exact float range",
                                ));
                            }
                            let left = numeric(value)?;
                            let right = numeric(reference)?;
                            if left.abs() > 9_007_199_254_740_992.0
                                || right.abs() > 9_007_199_254_740_992.0
                            {
                                return Err(Error::Invalid(
                                    "mixed numeric comparison exceeds exact float range",
                                ));
                            }
                            left.partial_cmp(&right)
                        }
                    }
                } else {
                    return Err(invalid());
                }
                .ok_or_else(invalid)?;
            Ok(match comparison {
                Comparison::Lt { .. } => ordering.is_lt(),
                Comparison::Le { .. } => !ordering.is_gt(),
                Comparison::Gt { .. } => ordering.is_gt(),
                _ => !ordering.is_lt(),
            })
        }
        Comparison::Approx {
            reference,
            threshold,
        } => {
            if !reference.is_finite() || !threshold.is_finite() || *threshold < 0.0 {
                return Err(invalid());
            }
            Ok(numeric(value).is_ok_and(|value| (value - reference).abs() <= *threshold))
        }
        Comparison::Range {
            minimum,
            maximum,
            minimum_inclusive,
            maximum_inclusive,
        } => {
            if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum {
                return Err(invalid());
            }
            let value = value.as_f64().or_else(|| value.as_bool().map(|value| f64::from(u8::from(value)))).ok_or_else(invalid)?;
            Ok((if *minimum_inclusive {
                value >= *minimum
            } else {
                value > *minimum
            }) && (if *maximum_inclusive {
                value <= *maximum
            } else {
                value < *maximum
            }))
        }
        Comparison::StrListEq { reference } => Ok(text(value)?
            .trim_matches(['\'', '"'])
            .split(',')
            .eq(reference.iter().map(String::as_str))),
        Comparison::StrSetEq { reference } => Ok(text(value)?
            .trim_matches(['\'', '"'])
            .split(',')
            .collect::<BTreeSet<_>>()
            == reference.iter().map(String::as_str).collect()),
    }
}

fn fuzzy(left: &str, right: &str) -> Result<f64, Error> {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.len() > 4096 || right.len() > 4096 {
        return Err(Error::Limit("fuzzy evaluator text"));
    }
    if left.is_empty() && right.is_empty() {
        return Ok(1.0);
    }
    let mut row = vec![0usize; right.len() + 1];
    for left_char in &left {
        let mut diagonal = 0;
        for (index, right_char) in right.iter().enumerate() {
            let above = row[index + 1];
            row[index + 1] = if left_char == right_char {
                diagonal + 1
            } else {
                above.max(row[index])
            };
            diagonal = above;
        }
    }
    let maximum = left.len() + right.len();
    let distance = maximum - 2 * row[right.len()];
    Ok(((1.0 - distance as f64 / maximum as f64) * 100.0) / 100.0)
}

pub fn evaluate(metric: &Metric, evidence: Option<&Value>) -> Result<Evaluation, Error> {
    if serde_json::to_vec(metric).map_err(|_| invalid())?.len() > MAX_INPUT_BYTES {
        return Err(Error::Limit("general evaluator rules"));
    }
    let Some(value) = evidence else {
        return Ok(Evaluation {
            outcome: "unknown",
            reward: None,
        });
    };
    let evidence_bytes = serde_json::to_vec(value).map_err(|_| invalid())?.len();
    if evidence_bytes > MAX_INPUT_BYTES {
        return Err(Error::Limit("general evaluator evidence"));
    }
    let rule_units = match metric {
        Metric::IncludeExclude { include, exclude } => include.len().saturating_add(exclude.len()),
        Metric::JsonInclusion { expected } => expected.values().map(|value| value.as_array().map_or(1, Vec::len)).fold(0usize, usize::saturating_add),
        Metric::JsonSubset { expected } => expected.len(),
        Metric::JsonRules { expect, unexpect } => expect.iter().chain(unexpect).map(|rule| 1usize.saturating_add(rule.key.len())).fold(0usize,usize::saturating_add),
        Metric::Records { expect, unexpect } => expect.iter().chain(unexpect).map(Map::len).fold(0usize,usize::saturating_add),
        _ => 1,
    };
    if evidence_bytes.saturating_add(1).saturating_mul(rule_units.saturating_add(1)) > 64 * 1024 * 1024 {
        return Err(Error::Limit("general evaluator work budget"));
    }
    let matched = match metric {
        Metric::IncludeExclude { include, exclude } => {
            let value = text(value)?;
            include.iter().all(|part| value.contains(part))
                && exclude.iter().all(|part| !value.contains(part))
        }
        Metric::LiteralString {
            expected,
            ignore_case,
        } => {
            let value = text(value)?;
            if *ignore_case {
                value.to_lowercase() == expected.to_lowercase()
            } else {
                value == expected
            }
        }
        Metric::LiteralList {
            expected,
            ignore_case,
        } => {
            let value = strings(value)?;
            value.len() == expected.len()
                && value.iter().zip(expected).all(|(value, expected)| {
                    if *ignore_case {
                        value.to_lowercase() == expected.to_lowercase()
                    } else {
                        *value == expected
                    }
                })
        }
        Metric::Membership { expected } => match value {
            Value::Array(items) => items.iter().any(|item| equal(item, expected)),
            Value::Object(items) => items.contains_key(text(expected)?),
            Value::String(value) => value.contains(text(expected)?),
            _ => return Err(invalid()),
        },
        Metric::JsonSubset { expected } => {
            let value = value.as_object().ok_or_else(invalid)?;
            expected
                .iter()
                .all(|(key, expected)| value.get(key).is_some_and(|actual| equal(actual, expected)))
        }
        Metric::JsonInclusion { expected } => {
            let value = value.as_object().ok_or_else(invalid)?;
            expected
                .iter()
                .all(|(key, expected)| match (value.get(key), expected) {
                    (Some(Value::Array(actual)), Value::Array(expected)) => expected
                        .iter()
                        .any(|item| actual.iter().any(|actual| equal(actual, item))),
                    (Some(Value::String(actual)), Value::String(expected)) => {
                        actual.contains(expected)
                    }
                    _ => false,
                })
        }
        Metric::JsonRules { expect, unexpect } => {
            if expect.len() + unexpect.len() > 1024
                || expect
                    .iter()
                    .chain(unexpect)
                    .any(|rule| rule.key.len() > 64)
            {
                return Err(Error::Limit("general evaluator JSON rules"));
            }
            let mut matches = true;
            for rule in expect {
                matches &= match value_at(value, &rule.key) {
                    Some(value) => compare(value, &rule.comparison)?,
                    None => false,
                };
            }
            for rule in unexpect {
                matches &= !compare(
                    value_at(value, &rule.key).unwrap_or(&Value::Null),
                    &rule.comparison,
                )?;
            }
            matches
        }
        Metric::FuzzyText { expected } => {
            return Ok(scored(fuzzy(text(value)?, expected)?));
        }
        Metric::TextEqual {
            expected,
            ignore_whitespace,
        } => {
            let value = text(value)?;
            if *ignore_whitespace {
                value
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .eq(expected.chars().filter(|c| !c.is_whitespace()))
            } else {
                value == expected
            }
        }
        Metric::Records { expect, unexpect } => {
            let records = value
                .as_array()
                .ok_or_else(invalid)?
                .iter()
                .map(|item| item.as_object().ok_or_else(invalid))
                .collect::<Result<Vec<_>, _>>()?;
            let matches = |record: &Map<String, Value>, rule: &Map<String, Value>| {
                rule.iter()
                    .all(|(key, value)| record.get(key).is_some_and(|actual| equal(actual, value)))
            };
            expect
                .iter()
                .all(|rule| records.iter().any(|record| matches(record, rule)))
                && unexpect
                    .iter()
                    .all(|rule| records.iter().all(|record| !matches(record, rule)))
        }
    };
    Ok(scored(if matched { 1.0 } else { 0.0 }))
}

fn scored(reward: f64) -> Evaluation {
    Evaluation {
        outcome: if reward == 1.0 {
            "success"
        } else if reward == 0.0 {
            "failure"
        } else {
            "partial"
        },
        reward: Some(reward),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn pinned_general_metric_gold_cases() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../fixtures/general-metrics.json")).unwrap();
        for case in cases {
            let metric = serde_json::from_value(case["metric"].clone()).unwrap();
            let result = evaluate(&metric, Some(&case["evidence"])).unwrap();
            assert_eq!(result.reward, case["reward"].as_f64(), "{case}");
        }
    }
    #[test]
    fn absent_evidence_stays_unknown_and_invalid_rules_are_errors() {
        let metric = Metric::LiteralString {
            expected: "saved".into(),
            ignore_case: false,
        };
        assert_eq!(evaluate(&metric, None).unwrap().outcome, "unknown");
        assert!(evaluate(&metric, Some(&json!(true))).is_err());
        assert!(
            compare(
                &json!(1),
                &Comparison::Approx {
                    reference: 1.0,
                    threshold: -1.0
                }
            )
            .is_err()
        );
        assert!(
            compare(
                &json!(1),
                &Comparison::Range {
                    minimum: 2.0,
                    maximum: 1.0,
                    minimum_inclusive: true,
                    maximum_inclusive: true
                }
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Metric>(
                json!({"kind":"run_sqlite3","sql":"DELETE FROM secret"})
            )
            .is_err()
        );
    }
    #[test]
    fn unicode_fuzzy_work_is_bounded() {
        assert!(fuzzy(&"a".repeat(4097), "a").is_err());
        assert_eq!(fuzzy("abc", "xyz").unwrap(), 0.0);
        assert_eq!(fuzzy("abc", "").unwrap(), 0.0);
        let metric = Metric::IncludeExclude { include: vec!["a".into(); 1024], exclude: Vec::new() };
        assert!(evaluate(&metric,Some(&Value::String("a".repeat(70000)))).is_err());
    }
    #[test]
    fn numeric_identity_never_collapses_large_integers() {
        assert!(!equal(
            &json!(9007199254740993_u64),
            &json!(9007199254740992.0)
        ));
        assert!(equal(
            &json!(9007199254740992_u64),
            &json!(9007199254740992.0)
        ));
        assert!(equal(&json!(1), &json!(1.0)));
        assert!(equal(&json!({"x":true}), &json!({"x":1})));
        assert!(!equal(
            &json!(u64::MAX),
            &json!(18_446_744_073_709_551_616.0)
        ));
        assert!(!equal(
            &json!(i64::MAX),
            &json!(9_223_372_036_854_775_808.0)
        ));
        assert!(
            compare(
                &json!(9007199254740993_u64),
                &Comparison::Gt {
                    reference: json!(9007199254740992.0)
                }
            )
            .is_err()
        );
    }
}
